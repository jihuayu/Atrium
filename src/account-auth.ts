import { ApiError } from "./error";
import type { AppContext, Env } from "./types";

export type AccountAuthRoute = "account" | "github";

export interface AccountOAuthState {
  redirectUri: string;
  userState: string;
  nonce: string;
}

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

export function accountIssuer(env: Env): string {
  return (env.ACCOUNT_ISSUER?.trim() || "https://account.jihuayu.com").replace(/\/+$/, "");
}

export function accountJwksUrl(env: Env): string {
  return `${accountIssuer(env)}/oauth/jwks`;
}

export function accountClientId(env: Env): string {
  const clientId = env.ACCOUNT_CLIENT_ID?.trim();
  if (!clientId) throw new ApiError(501, "Jihuayu Account login is not enabled on this server");
  return clientId;
}

export function accountScope(env: Env): string {
  return env.ACCOUNT_SCOPE?.trim() || "openid profile email";
}

export function accountCallbackUri(ctx: AppContext, route: AccountAuthRoute): string {
  return ctx.env.ACCOUNT_REDIRECT_URI?.trim() || `${ctx.baseUrl}/api/v1/auth/${route}/callback`;
}

export async function buildAccountAuthorizeLocation(
  ctx: AppContext,
  route: AccountAuthRoute,
  finalRedirectUri: string,
  userState: string
): Promise<string> {
  const callbackUri = accountCallbackUri(ctx, route);
  const nonce = randomToken();
  const state = await buildAccountOAuthState(ctx, {
    redirectUri: finalRedirectUri,
    userState,
    nonce
  });
  const url = new URL(`${accountIssuer(ctx.env)}/oauth/authorize`);
  url.searchParams.set("response_type", "code");
  url.searchParams.set("client_id", accountClientId(ctx.env));
  url.searchParams.set("redirect_uri", callbackUri);
  url.searchParams.set("scope", accountScope(ctx.env));
  url.searchParams.set("state", state);
  url.searchParams.set("nonce", nonce);
  return url.toString();
}

export async function exchangeAccountAuthorizationCode(ctx: AppContext, code: string, callbackUri: string): Promise<string> {
  const form = new URLSearchParams({
    grant_type: "authorization_code",
    client_id: accountClientId(ctx.env),
    code,
    redirect_uri: callbackUri
  });
  const clientSecret = ctx.env.ACCOUNT_CLIENT_SECRET?.trim();
  if (clientSecret) form.set("client_secret", clientSecret);

  const response = await fetch(`${accountIssuer(ctx.env)}/oauth/token`, {
    method: "POST",
    headers: {
      Accept: "application/json",
      "Content-Type": "application/x-www-form-urlencoded"
    },
    body: form
  });
  const payload = (await response.json().catch(() => ({}))) as { id_token?: string; error?: string; message?: string };
  if (!response.ok) {
    throw new ApiError(response.status, payload.message || payload.error || `Jihuayu Account OAuth error: ${response.status}`);
  }
  if (!payload.id_token) {
    throw ApiError.badRequest("Jihuayu Account token response did not include id_token");
  }
  return payload.id_token;
}

export async function buildAccountOAuthState(ctx: AppContext, state: AccountOAuthState): Promise<string> {
  const payload = JSON.stringify({
    ts: Math.floor(Date.now() / 1000),
    redirect_uri: state.redirectUri,
    state: state.userState,
    nonce: state.nonce
  });
  const signature = await hmacHex(ctx.jwtSecret, payload);
  return base64UrlEncode(textEncoder.encode(`${payload}\n${signature}`));
}

export async function verifyAccountOAuthState(ctx: AppContext, stateToken: string): Promise<AccountOAuthState> {
  let combined = "";
  try {
    combined = textDecoder.decode(base64UrlDecode(stateToken));
  } catch {
    throw ApiError.badRequest("invalid OAuth state");
  }
  const [payload, signature] = combined.split("\n");
  if (!payload || !signature) throw ApiError.badRequest("invalid OAuth state format");
  if ((await hmacHex(ctx.jwtSecret, payload)) !== signature) throw ApiError.badRequest("OAuth state signature mismatch");

  const parsed = parseStatePayload(payload);
  if (Math.floor(Date.now() / 1000) - parsed.ts > 600) throw ApiError.badRequest("OAuth state expired");
  return {
    redirectUri: parsed.redirectUri,
    userState: parsed.userState,
    nonce: parsed.nonce
  };
}

export function redirectWithUserState(redirectUri: string, userState: string): string {
  if (!userState) return redirectUri;
  const url = new URL(redirectUri);
  url.searchParams.set("state", userState);
  return url.toString();
}

async function hmacHex(secret: Uint8Array, message: string): Promise<string> {
  const key = await crypto.subtle.importKey("raw", secret, { name: "HMAC", hash: "SHA-256" }, false, ["sign"]);
  const sig = await crypto.subtle.sign("HMAC", key, textEncoder.encode(message));
  return [...new Uint8Array(sig)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function parseStatePayload(payload: string): { ts: number; redirectUri: string; userState: string; nonce: string } {
  try {
    const parsed = JSON.parse(payload) as { ts?: number; redirect_uri?: string; state?: string; nonce?: string };
    if (typeof parsed.ts !== "number" || typeof parsed.redirect_uri !== "string") {
      throw new Error("invalid state payload");
    }
    return {
      ts: parsed.ts,
      redirectUri: parsed.redirect_uri,
      userState: typeof parsed.state === "string" ? parsed.state : "",
      nonce: typeof parsed.nonce === "string" ? parsed.nonce : ""
    };
  } catch {
    const parts = payload.split("|");
    const ts = Number.parseInt(parts[0] ?? "", 10);
    if (!Number.isFinite(ts) || !parts[1]) throw ApiError.badRequest("invalid OAuth state payload");
    return {
      ts,
      redirectUri: parts[1],
      userState: parts[2] ?? "",
      nonce: ""
    };
  }
}

function randomToken(byteLength = 32): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return base64UrlEncode(bytes);
}

function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) binary += String.fromCharCode(byte);
  return btoa(binary).replaceAll("+", "-").replaceAll("/", "_").replaceAll("=", "");
}

function base64UrlDecode(value: string): Uint8Array {
  const base64 = value.replace(/-/g, "+").replace(/_/g, "/").padEnd(Math.ceil(value.length / 4) * 4, "=");
  const binary = atob(base64);
  const bytes = new Uint8Array(binary.length);
  for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
  return bytes;
}
