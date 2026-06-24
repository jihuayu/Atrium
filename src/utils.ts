import { createRemoteJWKSet, jwtVerify, SignJWT } from "jose";
import { micromark } from "micromark";
import { gfm, gfmHtml } from "micromark-extension-gfm";
import { ApiError } from "./error";
import type { AuthUser, PublicUser, ReactionCounts } from "./types";
import { EMPTY_REACTION_COUNTS } from "./types";

export const ACCESS_COOKIE = "atrium_access";
export const REFRESH_COOKIE = "atrium_refresh";
const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

export function parseSecret(value: string | undefined): Uint8Array {
  if (!value) return new Uint8Array();
  try {
    return binaryToBytes(atob(padBase64(value.replace(/-/g, "+").replace(/_/g, "/"))));
  } catch {
    return textEncoder.encode(value);
  }
}

export function parseToken(header: string | null | undefined): string | null {
  const value = header?.trim();
  if (!value) return null;
  if (value.startsWith("token ")) return value.slice(6).trim();
  if (value.startsWith("Bearer ")) return value.slice(7).trim();
  return null;
}

export function bearerFromHeader(header: string | null | undefined): string | null {
  if (!header) return null;
  const token = parseToken(header);
  if (!token) throw ApiError.unauthorized();
  return token;
}

export async function sha256Hex(value: string): Promise<string> {
  const digest = await crypto.subtle.digest("SHA-256", textEncoder.encode(value));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

export function toPublicUser(user: AuthUser, includeEmail = false): PublicUser {
  return {
    id: user.id,
    login: user.login,
    avatar_url: user.avatar_url,
    ...(includeEmail ? { email: user.email } : {})
  };
}

export function toIso(value: string | null | undefined): string | null {
  if (!value) return null;
  if (value.includes("T") && value.endsWith("Z")) return value;
  return `${value.replace(" ", "T")}Z`;
}

export function timestampSeconds(): number {
  return Math.floor(Date.now() / 1000);
}

export function isoNow(): string {
  return new Date().toISOString().replace(/\.\d{3}Z$/, "Z");
}

export function renderMarkdown(input: string): string {
  return micromark(input, {
    extensions: [gfm()],
    htmlExtensions: [gfmHtml()],
    allowDangerousHtml: false
  });
}

export function base64Std(input: string): string {
  return btoa(bytesToBinary(textEncoder.encode(input)));
}

export function base64UrlJson(value: unknown): string {
  return base64Std(JSON.stringify(value)).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/g, "");
}

export function decodeBase64UrlJson<T>(value: string, message = "invalid cursor"): T {
  try {
    const base64 = padBase64(value.replace(/-/g, "+").replace(/_/g, "/"));
    return JSON.parse(textDecoder.decode(binaryToBytes(atob(base64)))) as T;
  } catch {
    throw ApiError.badRequest(message);
  }
}

export function encodeCursor(id: number): string {
  return base64UrlJson({ id });
}

export function decodeCursor(cursor: string): number {
  const payload = decodeBase64UrlJson<{ id?: number }>(cursor);
  if (typeof payload.id !== "number") throw ApiError.badRequest("invalid cursor");
  return payload.id;
}

export function buildSetCookie(name: string, value: string, maxAgeSecs: number, secure: boolean): string {
  return `${name}=${value}; Path=/; HttpOnly; SameSite=Lax; Max-Age=${maxAgeSecs}${secure ? "; Secure" : ""}`;
}

export function clearCookie(name: string, secure: boolean): string {
  return `${name}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0${secure ? "; Secure" : ""}`;
}

export function cookieValue(header: string | null | undefined, name: string): string | null {
  if (!header) return null;
  for (const part of header.split(";")) {
    const [key, ...rest] = part.trim().split("=");
    if (key === name) return rest.join("=").trim();
  }
  return null;
}

export function secureFromBaseUrl(baseUrl: string): boolean {
  return baseUrl.startsWith("https://");
}

export async function signJwt(
  claims: Record<string, unknown>,
  secret: Uint8Array
): Promise<string> {
  if (secret.length < 16) throw ApiError.internal("jwt secret is too short");
  return new SignJWT(claims)
    .setProtectedHeader({ alg: "HS256", typ: "JWT" })
    .sign(secret);
}

export async function verifyAtriumJwt<T extends Record<string, unknown>>(
  token: string,
  secret: Uint8Array
): Promise<T> {
  try {
    const { protectedHeader, payload } = await jwtVerify(token, secret, {
      algorithms: ["HS256"],
      typ: "JWT"
    });
    if (protectedHeader.alg !== "HS256") throw ApiError.unauthorized();
    return payload as T;
  } catch (error) {
    if (error instanceof ApiError) throw error;
    throw ApiError.unauthorized();
  }
}

export async function verifyProviderJwt(
  token: string,
  jwksUrl: string,
  issuer: string,
  audience: string
): Promise<Record<string, unknown>> {
  try {
    const jwks = createRemoteJWKSet(new URL(jwksUrl));
    const { payload } = await jwtVerify(token, jwks, { issuer, audience });
    return payload;
  } catch {
    throw ApiError.unauthorized();
  }
}

export function parseReactionCounts(raw: string | null | undefined): ReactionCounts {
  if (!raw) return { ...EMPTY_REACTION_COUNTS };
  try {
    const parsed = { ...EMPTY_REACTION_COUNTS, ...JSON.parse(raw) } as ReactionCounts;
    parsed.total =
      parsed.like + parsed.dislike + parsed.heart + parsed.laugh + parsed.hooray + parsed.confused + parsed.rocket + parsed.eyes;
    return parsed;
  } catch {
    return { ...EMPTY_REACTION_COUNTS };
  }
}

export function normalizePagination(page?: string | null, perPage?: string | null): { page: number; perPage: number; offset: number } {
  const p = Math.max(1, Number.parseInt(page ?? "1", 10) || 1);
  const pp = Math.min(100, Math.max(1, Number.parseInt(perPage ?? "30", 10) || 30));
  return { page: p, perPage: pp, offset: (p - 1) * pp };
}

export function buildLinkHeader(baseUrl: string, path: string, page: number, perPage: number, total: number): string | null {
  if (perPage <= 0) return null;
  const lastPage = Math.max(1, Math.ceil(total / perPage));
  if (lastPage <= 1) return null;
  const links: string[] = [];
  if (page < lastPage) links.push(`<${baseUrl}${path}?page=${page + 1}&per_page=${perPage}>; rel="next"`);
  if (page > 1) links.push(`<${baseUrl}${path}?page=${page - 1}&per_page=${perPage}>; rel="prev"`);
  links.push(`<${baseUrl}${path}?page=1&per_page=${perPage}>; rel="first"`);
  links.push(`<${baseUrl}${path}?page=${lastPage}&per_page=${perPage}>; rel="last"`);
  return links.join(", ");
}

function padBase64(value: string): string {
  return value.padEnd(Math.ceil(value.length / 4) * 4, "=");
}

function bytesToBinary(bytes: Uint8Array): string {
  let out = "";
  for (let i = 0; i < bytes.length; i += 0x8000) {
    out += String.fromCharCode(...bytes.subarray(i, i + 0x8000));
  }
  return out;
}

function binaryToBytes(binary: string): Uint8Array {
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
