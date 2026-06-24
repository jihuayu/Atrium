import { compactDecrypt, importJWK, type JWK } from "jose";
import { ApiError } from "./error";
import type { AppContext } from "./types";

export const ENCRYPTED_FIELD_PREFIX = "enc:jwe:";
export const DISCOVERY_JWE_ALG = "RSA-OAEP-256";
export const DISCOVERY_JWE_ENC = "A256GCM";

const DISCOVERY_PATH = "/.well-known/atrium.json";
const TXT_RECORD_PREFIX = "atrium-site=";
const MAX_DISCOVERY_BYTES = 16 * 1024;
const DISCOVERY_TIMEOUT_MS = 2500;
const textDecoder = new TextDecoder();

export type DiscoverySource = "well-known" | "dns-txt";
export type DiscoveryFailureStatus = "not_found" | "invalid" | "error" | "conflict";

export interface DiscoveryMetadata {
  origin: string;
  websiteKey: string;
  name: string;
  adminEmails: string[];
  contactEmail: string | null;
  source: DiscoverySource;
}

export interface DiscoveryFailure {
  status: DiscoveryFailureStatus;
  source: DiscoverySource | null;
  error: string;
}

export interface DiscoveryLookupResult {
  metadata: DiscoveryMetadata | null;
  failure: DiscoveryFailure;
}

export class DiscoveryDocumentError extends Error {
  readonly failureStatus: Exclude<DiscoveryFailureStatus, "not_found" | "conflict">;

  constructor(message: string, failureStatus: Exclude<DiscoveryFailureStatus, "not_found" | "conflict"> = "invalid") {
    super(message);
    this.name = "DiscoveryDocumentError";
    this.failureStatus = failureStatus;
  }
}

export function isEncryptedDiscoveryValue(value: unknown): value is string {
  return typeof value === "string" && value.startsWith(ENCRYPTED_FIELD_PREFIX);
}

export async function discoveryPublicKeyResponse(ctx: AppContext) {
  const jwk = discoveryPublicJwk(ctx);
  const kid = discoveryKeyId(ctx, jwk);
  const publicJwk = publicOnlyJwk(jwk);
  if (!publicJwk.kty) throw ApiError.internal("ATRIUM_DISCOVERY_PUBLIC_JWK is invalid");
  if (kid) publicJwk.kid = kid;
  publicJwk.alg = DISCOVERY_JWE_ALG;
  publicJwk.key_ops = ["encrypt"];
  return {
    kid: kid ?? null,
    alg: DISCOVERY_JWE_ALG,
    enc: DISCOVERY_JWE_ENC,
    jwk: publicJwk
  };
}

export async function discoverOriginMetadata(ctx: AppContext, origin: string): Promise<DiscoveryLookupResult> {
  let expectedOrigin: URL;
  try {
    expectedOrigin = new URL(origin);
  } catch {
    return failure("invalid", null, "invalid origin");
  }
  if (expectedOrigin.protocol !== "https:") {
    return failure("not_found", null, "discovery requires https origin");
  }

  const failures: DiscoveryFailure[] = [];
  const fileText = await readWellKnownDiscovery(ctx, expectedOrigin).catch((error) => {
    failures.push(failure("error", "well-known", errorMessage(error)).failure);
    return null;
  });
  if (fileText != null) {
    const metadata = await parseCandidate(ctx, fileText, origin, "well-known", failures);
    if (metadata) return success(metadata);
  }

  const txtPayloads = await readDnsTxtDiscovery(ctx, expectedOrigin.hostname).catch((error) => {
    failures.push(failure("error", "dns-txt", errorMessage(error)).failure);
    return [];
  });
  for (const payload of txtPayloads) {
    const metadata = await parseCandidate(ctx, payload, origin, "dns-txt", failures);
    if (metadata) return success(metadata);
  }

  const preferred = failures.find((item) => item.status === "invalid") ?? failures.find((item) => item.status === "error");
  return { metadata: null, failure: preferred ?? failure("not_found", null, "discovery metadata not found").failure };
}

export async function parseDiscoveryDocument(
  ctx: AppContext,
  raw: unknown,
  expectedOrigin: string,
  source: DiscoverySource
): Promise<DiscoveryMetadata> {
  if (!isRecord(raw)) throw new DiscoveryDocumentError("document must be a JSON object");
  const document = await decryptFlatDiscoveryFields(ctx, raw);
  if (document.atrium !== "v1") throw new DiscoveryDocumentError("atrium must be v1");

  const origin = requireString(document.origin, "origin");
  const normalizedOrigin = normalizeDiscoveryOrigin(origin);
  if (normalizedOrigin !== expectedOrigin) throw new DiscoveryDocumentError("origin does not match referer origin");

  const originUrl = new URL(expectedOrigin);
  const websiteKey = document.website_key == null ? normalizeDiscoveryKey(originUrl.hostname) : normalizeDiscoveryKey(document.website_key);
  const name = document.name == null ? websiteKey : requireString(document.name, "name").trim();
  if (!name || name.length > 160) throw new DiscoveryDocumentError("name is invalid");

  return {
    origin: normalizedOrigin,
    websiteKey,
    name,
    adminEmails: normalizeAdminEmails(document.admin_emails),
    contactEmail: document.contact_email == null ? null : normalizeEmail(requireString(document.contact_email, "contact_email"), "contact_email"),
    source
  };
}

export async function decryptFlatDiscoveryFields(ctx: AppContext, input: Record<string, unknown>): Promise<Record<string, unknown>> {
  const output: Record<string, unknown> = { ...input };
  for (const [field, value] of Object.entries(input)) {
    if (!isEncryptedDiscoveryValue(value)) continue;
    output[field] = await decryptDiscoveryField(ctx, field, value.slice(ENCRYPTED_FIELD_PREFIX.length));
  }
  return output;
}

export function parseAtriumTxtPayloadsFromDoh(payload: unknown): string[] {
  if (!isRecord(payload) || !Array.isArray(payload.Answer)) return [];
  const out: string[] = [];
  for (const answer of payload.Answer) {
    if (!isRecord(answer)) continue;
    if (Number(answer.type) !== 16 || typeof answer.data !== "string") continue;
    const joined = parseDnsTxtData(answer.data).join("");
    if (joined.startsWith(TXT_RECORD_PREFIX)) out.push(joined.slice(TXT_RECORD_PREFIX.length));
  }
  return out;
}

export function parseDnsTxtData(data: string): string[] {
  const values: string[] = [];
  let index = 0;
  while (index < data.length) {
    while (index < data.length && /\s/.test(data[index]!)) index += 1;
    if (index >= data.length) break;
    if (data[index] !== '"') {
      if (values.length === 0) return [data.trim()];
      throw new DiscoveryDocumentError("invalid TXT record data");
    }
    index += 1;
    let value = "";
    let closed = false;
    while (index < data.length) {
      const char = data[index]!;
      index += 1;
      if (char === '"') {
        closed = true;
        break;
      }
      if (char === "\\") {
        const decimal = data.slice(index, index + 3);
        if (/^\d{3}$/.test(decimal)) {
          value += String.fromCharCode(Number(decimal));
          index += 3;
        } else if (index < data.length) {
          value += data[index]!;
          index += 1;
        } else {
          value += "\\";
        }
      } else {
        value += char;
      }
    }
    if (!closed) throw new DiscoveryDocumentError("unterminated TXT string");
    values.push(value);
  }
  return values;
}

async function parseCandidate(
  ctx: AppContext,
  text: string,
  expectedOrigin: string,
  source: DiscoverySource,
  failures: DiscoveryFailure[]
): Promise<DiscoveryMetadata | null> {
  try {
    return await parseDiscoveryDocument(ctx, JSON.parse(text), expectedOrigin, source);
  } catch (error) {
    const status = error instanceof DiscoveryDocumentError ? error.failureStatus : "invalid";
    failures.push(failure(status, source, errorMessage(error)).failure);
    return null;
  }
}

async function readWellKnownDiscovery(ctx: AppContext, origin: URL): Promise<string | null> {
  const mocked = mockedWellKnownText(ctx, origin.origin);
  if (mocked !== undefined) return mocked;
  if (ctx.env.ATRIUM_TEST_DISCOVERY_WELL_KNOWN) return null;

  const url = new URL(DISCOVERY_PATH, origin.origin);
  const response = await fetchSameOriginWithTimeout(url, origin.origin);
  if (response.status === 404 || response.status === 410) return null;
  if (!response.ok) throw new Error(`well-known returned ${response.status}`);
  return readResponseTextLimited(response, MAX_DISCOVERY_BYTES);
}

async function readDnsTxtDiscovery(ctx: AppContext, hostname: string): Promise<string[]> {
  const mocked = mockedDnsTxtPayloads(ctx, hostname);
  if (mocked !== undefined) return mocked;
  if (ctx.env.ATRIUM_TEST_DISCOVERY_DNS_TXT) return [];

  const url = new URL("https://cloudflare-dns.com/dns-query");
  url.searchParams.set("name", `_atrium.${hostname}`);
  url.searchParams.set("type", "TXT");
  const response = await fetchWithTimeout(url, {
    headers: { Accept: "application/dns-json" },
    redirect: "manual"
  });
  if (!response.ok) throw new Error(`dns query returned ${response.status}`);
  const text = await readResponseTextLimited(response, MAX_DISCOVERY_BYTES);
  return parseAtriumTxtPayloadsFromDoh(JSON.parse(text));
}

async function fetchSameOriginWithTimeout(url: URL, expectedOrigin: string): Promise<Response> {
  let current = url;
  for (let i = 0; i < 3; i += 1) {
    const response = await fetchWithTimeout(current, {
      headers: { Accept: "application/json" },
      redirect: "manual"
    });
    if (response.status < 300 || response.status >= 400) return response;

    const location = response.headers.get("Location");
    if (!location) return response;
    const next = new URL(location, current);
    if (next.protocol !== "https:" || next.origin !== expectedOrigin) {
      throw new Error("cross-origin discovery redirect is not allowed");
    }
    current = next;
  }
  throw new Error("too many discovery redirects");
}

async function fetchWithTimeout(url: URL, init: RequestInit): Promise<Response> {
  const controller = new AbortController();
  const timeout = setTimeout(() => controller.abort(), DISCOVERY_TIMEOUT_MS);
  try {
    return await fetch(url.toString(), { ...init, signal: controller.signal });
  } finally {
    clearTimeout(timeout);
  }
}

async function readResponseTextLimited(response: Response, maxBytes: number): Promise<string> {
  if (!response.body) return "";
  const reader = response.body.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  while (true) {
    const { done, value } = await reader.read();
    if (done) break;
    if (!value) continue;
    total += value.byteLength;
    if (total > maxBytes) {
      await reader.cancel("response too large");
      throw new Error("discovery response too large");
    }
    chunks.push(value);
  }
  const bytes = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    bytes.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return textDecoder.decode(bytes);
}

async function decryptDiscoveryField(ctx: AppContext, field: string, compactJwe: string): Promise<unknown> {
  const privateJwk = discoveryPrivateJwk(ctx);
  const key = await importJWK(privateJwk, DISCOVERY_JWE_ALG);
  try {
    const { plaintext, protectedHeader } = await compactDecrypt(compactJwe, key, {
      keyManagementAlgorithms: [DISCOVERY_JWE_ALG],
      contentEncryptionAlgorithms: [DISCOVERY_JWE_ENC]
    });
    if (protectedHeader.alg !== DISCOVERY_JWE_ALG || protectedHeader.enc !== DISCOVERY_JWE_ENC) {
      throw new DiscoveryDocumentError(`${field} uses unsupported encryption`);
    }
    const expectedKid = discoveryKeyId(ctx, privateJwk);
    if (expectedKid && protectedHeader.kid !== expectedKid) {
      throw new DiscoveryDocumentError(`${field} uses an unknown key id`);
    }
    return JSON.parse(textDecoder.decode(plaintext));
  } catch (error) {
    if (error instanceof DiscoveryDocumentError) throw error;
    throw new DiscoveryDocumentError(`${field} could not be decrypted`);
  }
}

function discoveryPublicJwk(ctx: AppContext): JWK {
  const publicJwk = parseJwk(ctx.env.ATRIUM_DISCOVERY_PUBLIC_JWK);
  if (publicJwk) return publicJwk;
  const privateJwk = parseJwk(ctx.env.ATRIUM_DISCOVERY_PRIVATE_JWK);
  if (privateJwk) return privateJwk;
  throw ApiError.internal("ATRIUM_DISCOVERY_PUBLIC_JWK is not configured");
}

function discoveryPrivateJwk(ctx: AppContext): JWK {
  const jwk = parseJwk(ctx.env.ATRIUM_DISCOVERY_PRIVATE_JWK);
  if (!jwk) throw new DiscoveryDocumentError("ATRIUM_DISCOVERY_PRIVATE_JWK is not configured", "error");
  return jwk;
}

function parseJwk(raw: string | undefined): JWK | null {
  if (!raw) return null;
  try {
    const parsed = JSON.parse(raw) as unknown;
    if (!isRecord(parsed)) return null;
    return parsed as JWK;
  } catch {
    return null;
  }
}

function discoveryKeyId(ctx: AppContext, jwk: JWK): string | null {
  return trimmedOrNull(ctx.env.ATRIUM_DISCOVERY_KEY_ID) ?? trimmedOrNull(jwk.kid);
}

function publicOnlyJwk(jwk: JWK): JWK {
  const privateFields = new Set(["d", "p", "q", "dp", "dq", "qi", "oth", "k", "priv"]);
  const out: JWK = {};
  for (const [key, value] of Object.entries(jwk)) {
    if (!privateFields.has(key)) (out as Record<string, unknown>)[key] = value;
  }
  return out;
}

function normalizeDiscoveryOrigin(raw: string): string {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new DiscoveryDocumentError("origin is invalid");
  }
  if (url.protocol !== "https:") throw new DiscoveryDocumentError("origin must be https");
  url.hostname = url.hostname.toLowerCase();
  if (url.port === "443") url.port = "";
  url.pathname = "";
  url.search = "";
  url.hash = "";
  return url.origin;
}

function normalizeDiscoveryKey(value: unknown): string {
  const key = String(value ?? "").trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9_.-]{1,127}$/.test(key)) throw new DiscoveryDocumentError("website_key is invalid");
  return key;
}

function normalizeAdminEmails(value: unknown): string[] {
  if (value == null) return [];
  if (!Array.isArray(value)) throw new DiscoveryDocumentError("admin_emails must be an array");
  if (value.length > 20) throw new DiscoveryDocumentError("admin_emails has too many entries");
  const seen = new Set<string>();
  const emails: string[] = [];
  for (const item of value) {
    const email = normalizeEmail(requireString(item, "admin_emails"), "admin_emails");
    if (!seen.has(email)) {
      seen.add(email);
      emails.push(email);
    }
  }
  return emails;
}

function normalizeEmail(raw: string, field: string): string {
  const email = raw.trim().toLowerCase();
  if (email.length > 254 || !/^[^@\s]+@[^@\s]+\.[^@\s]+$/.test(email)) {
    throw new DiscoveryDocumentError(`${field} is invalid`);
  }
  return email;
}

function requireString(value: unknown, field: string): string {
  if (typeof value !== "string") throw new DiscoveryDocumentError(`${field} must be a string`);
  return value;
}

function mockedWellKnownText(ctx: AppContext, origin: string): string | undefined {
  const raw = ctx.env.ATRIUM_TEST_DISCOVERY_WELL_KNOWN;
  if (!raw) return undefined;
  const value = parseMockMap(raw)[origin];
  if (value === undefined) return undefined;
  return typeof value === "string" ? value : JSON.stringify(value);
}

function mockedDnsTxtPayloads(ctx: AppContext, hostname: string): string[] | undefined {
  const raw = ctx.env.ATRIUM_TEST_DISCOVERY_DNS_TXT;
  if (!raw) return undefined;
  const map = parseMockMap(raw);
  const value = map[hostname] ?? map[`_atrium.${hostname}`];
  if (value === undefined) return undefined;
  const records = Array.isArray(value) ? [value.map(String).join("")] : [String(value)];
  return records.map((record) => (record.startsWith(TXT_RECORD_PREFIX) ? record.slice(TXT_RECORD_PREFIX.length) : record));
}

function parseMockMap(raw: string): Record<string, unknown> {
  try {
    const parsed = JSON.parse(raw) as unknown;
    return isRecord(parsed) ? parsed : {};
  } catch {
    return {};
  }
}

function trimmedOrNull(value: unknown): string | null {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function success(metadata: DiscoveryMetadata): DiscoveryLookupResult {
  return { metadata, failure: failure("not_found", null, "").failure };
}

function failure(status: DiscoveryFailureStatus, source: DiscoverySource | null, error: string): DiscoveryLookupResult {
  return { metadata: null, failure: { status, source, error } };
}

function errorMessage(error: unknown): string {
  if (error instanceof Error) return error.message.slice(0, 240);
  return String(error).slice(0, 240);
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
