import { ApiError } from "./error";
import type { AppContext, Env } from "./types";

export const ACCOUNT_SSO_COOKIE = "__Secure-jihuayu_sso";

export interface AccountSessionUser {
  sub: string;
  username?: string;
  preferred_username?: string;
  handle?: string;
  displayName?: string;
  display_name?: string;
  name?: string;
  avatarUrl?: string;
  avatar_url?: string;
  picture?: string;
  email?: string;
}

interface AccountIntrospectionResponse {
  active?: boolean;
  user?: AccountSessionUser;
}

export function accountBaseUrl(env: Env): string {
  return (env.ACCOUNT_BASE_URL?.trim() || env.ACCOUNT_ISSUER?.trim() || "https://account.jihuayu.com").replace(/\/+$/, "");
}

export function accountAudience(env: Env): string {
  return env.ACCOUNT_AUDIENCE?.trim() || "atrium";
}

export function accountLoginLocation(env: Env, returnTo: string): string {
  const url = new URL(`${accountBaseUrl(env)}/login`);
  url.searchParams.set("return_to", returnTo);
  return url.toString();
}

export function redirectWithUserState(redirectUri: string, userState: string): string {
  if (!userState) return redirectUri;
  const url = new URL(redirectUri);
  url.searchParams.set("state", userState);
  return url.toString();
}

export async function introspectAccountCookie(ctx: AppContext, cookieHeader: string | null | undefined): Promise<AccountSessionUser | null> {
  if (!hasAccountSsoCookie(cookieHeader)) return null;

  const headers = new Headers({
    Accept: "application/json",
    "Content-Type": "application/json",
    Cookie: cookieHeader ?? ""
  });
  const internalSecret = ctx.env.ACCOUNT_INTERNAL_SECRET?.trim();
  if (internalSecret) headers.set("x-internal-secret", internalSecret);

  const response = await fetch(`${accountBaseUrl(ctx.env)}/internal/session/introspect`, {
    method: "POST",
    headers,
    body: JSON.stringify({ audience: accountAudience(ctx.env) })
  });
  const payload = (await response.json().catch(() => ({}))) as AccountIntrospectionResponse & { message?: string; error?: string };
  if (!response.ok) {
    throw new ApiError(response.status === 401 || response.status === 403 ? response.status : 502, payload.message || payload.error || "Account session introspection failed");
  }
  if (!payload.active || !payload.user?.sub) return null;
  return payload.user;
}

export function accountSessionDisplayName(user: AccountSessionUser): string {
  return firstPresent(user.displayName, user.display_name, user.name, user.email) || `account-${user.sub}`;
}

export function accountSessionAvatarUrl(user: AccountSessionUser): string {
  return firstPresent(user.avatarUrl, user.avatar_url, user.picture) || "";
}

function firstPresent(...values: Array<string | undefined>): string {
  return values.map((value) => value?.trim()).find((value): value is string => Boolean(value)) ?? "";
}

function hasAccountSsoCookie(cookieHeader: string | null | undefined): boolean {
  if (!cookieHeader) return false;
  return cookieHeader.split(";").some((part) => part.trim().startsWith(`${ACCOUNT_SSO_COOKIE}=`));
}
