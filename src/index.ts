import { Hono } from "hono";
import type { Context } from "hono";
import { accountLoginLocation, redirectWithUserState } from "./account-auth";
import { Database } from "./db";
import { renderDiscoveryGuide } from "./discovery-guide";
import { ApiError, asApiError } from "./error";
import * as svc from "./services";
import type { AppContext, AuthUser, Env } from "./types";
import {
  ACCESS_COOKIE,
  REFRESH_COOKIE,
  bearerFromHeader,
  buildSetCookie,
  clearCookie,
  cookieValue,
  parseSecret,
  parseToken,
  secureFromBaseUrl,
  toPublicUser
} from "./utils";

type Vars = { ctx: AppContext };
const app = new Hono<{ Bindings: Env; Variables: Vars }>();

app.use("*", async (c, next) => {
  if (c.req.method === "OPTIONS") {
    return addCors(c, new Response(null, { status: 204, headers: { Allow: "GET,POST,PUT,PATCH,DELETE,OPTIONS" } }));
  }
  try {
    c.set("ctx", await buildContext(c));
  } catch (error) {
    const apiError = asApiError(error);
    return addCors(c, json(apiError.nativeBody(), apiError.status));
  }
  await next();
  c.res = addCors(c, c.res);
});

app.get("/", (c) =>
  textResponse(
    `Atrium - native website/page/comment service

站点接入:
  1. 阅读完整接入说明: ${c.get("ctx").baseUrl.replace(/\/+$/, "")}/docs/discovery
  2. 在站点发布 https://<host>/.well-known/atrium.json，或添加 _atrium.<host> TXT
  3. 不需要声明 website_key；Atrium 会从当前页面 hostname 推导，origin 可省略
  4. 明文或 enc:jwe: 加密字段都支持；加密公钥见 /api/v1/discovery/public-key
  5. admin_emails 里的邮箱登录后会自动认领该 website admin 权限

Native API:
  GET    /docs/discovery

  POST   /api/v1/auth/account
  GET    /api/v1/auth/account/authorize
  GET    /api/v1/auth/account/callback
  POST   /api/v1/auth/refresh
  DELETE /api/v1/auth/session
  GET    /api/v1/auth/me

  GET    /api/v1/discovery/public-key

  POST   /api/v1/websites
  GET    /api/v1/websites
  GET    /api/v1/websites/{websiteKey}
  PATCH  /api/v1/websites/{websiteKey}
  GET    /api/v1/websites/{websiteKey}/admins
  POST   /api/v1/websites/{websiteKey}/admins
  DELETE /api/v1/websites/{websiteKey}/admins/{userId}

  PUT    /api/v1/websites/{websiteKey}/pages/{pageKey}
  GET    /api/v1/websites/{websiteKey}/pages/{pageKey}
  GET    /api/v1/websites/{websiteKey}/pages
  GET    /api/v1/websites/{websiteKey}/pages/{pageKey}/comments
  POST   /api/v1/websites/{websiteKey}/pages/{pageKey}/comments
  PATCH  /api/v1/websites/{websiteKey}/comments/{commentId}
  DELETE /api/v1/websites/{websiteKey}/comments/{commentId}
  PUT    /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}
  DELETE /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}

  GET    /api/v1/comments/current
  POST   /api/v1/comments/current
  GET    /api/v1/comments/current/replies
  PUT    /api/v1/comments/current/{commentId}/reactions/{content}
  DELETE /api/v1/comments/current/{commentId}/reactions/{content}

  GET    /api/v1/websites/{websiteKey}/admin/comments
  POST   /api/v1/websites/{websiteKey}/bans
  GET    /api/v1/websites/{websiteKey}/bans
  DELETE /api/v1/websites/{websiteKey}/bans/{userId}
`
  )
);

app.get("/docs/discovery", (c) => htmlResponse(renderDiscoveryGuide(c.get("ctx").baseUrl)));

app.post("/api/v1/auth/account", (c) =>
  native(c, async (ctx) => {
    if (!ctx.user) throw ApiError.unauthorized();
    ensureJwtSecret(ctx);
    const tokens = await svc.issueAtriumTokens(ctx, ctx.user);
    return withAuthCookies(ctx, json(tokens), tokens);
  })
);
app.get("/api/v1/auth/account/authorize", (c) => native(c, async (ctx) => accountAuthorize(c, ctx)));
app.get("/api/v1/auth/account/callback", (c) => native(c, async (ctx) => accountCallback(c, ctx)));
app.post("/api/v1/auth/refresh", (c) =>
  native(c, async (ctx) => {
    ensureJwtSecret(ctx);
    let token: string | null = null;
    const bodyText = await c.req.text();
    if (bodyText) {
      try {
        token = JSON.parse(bodyText).refresh_token ?? null;
      } catch {
        throw ApiError.badRequest("Invalid request body");
      }
    } else {
      token = bearerFromHeader(c.req.header("Authorization")) ?? cookieValue(c.req.header("Cookie"), REFRESH_COOKIE);
    }
    if (!token) throw ApiError.unauthorized();
    const tokens = await svc.refreshAtriumTokens(ctx, token);
    return withAuthCookies(ctx, json(tokens), tokens);
  })
);
app.delete("/api/v1/auth/session", (c) =>
  native(c, async (ctx) => {
    if (!ctx.user) throw ApiError.unauthorized();
    const response = empty();
    const secure = secureFromBaseUrl(ctx.baseUrl);
    response.headers.append("Set-Cookie", clearCookie(ACCESS_COOKIE, secure));
    response.headers.append("Set-Cookie", clearCookie(REFRESH_COOKIE, secure));
    return response;
  })
);
app.get("/api/v1/auth/me", (c) =>
  native(c, async (ctx) => {
    if (!ctx.user) throw ApiError.unauthorized();
    return json({ user: toPublicUser(ctx.user, true), super_admin: await svc.isSuperAdmin(ctx) });
  })
);

app.get("/api/v1/discovery/public-key", (c) => native(c, async (ctx) => json(await svc.getDiscoveryPublicKey(ctx))));

app.post("/api/v1/websites", (c) => native(c, async (ctx) => json(await svc.createWebsite(ctx, await bodyJson(c)), 201)));
app.get("/api/v1/websites", (c) => native(c, async (ctx) => json(await svc.listWebsites(ctx, query(c)))));
app.get("/api/v1/websites/:websiteKey", (c) => native(c, async (ctx) => json(await svc.getWebsiteResponse(ctx, c.req.param("websiteKey")))));
app.patch("/api/v1/websites/:websiteKey", (c) =>
  native(c, async (ctx) => json(await svc.updateWebsite(ctx, c.req.param("websiteKey"), await bodyJson(c))))
);
app.get("/api/v1/websites/:websiteKey/admins", (c) => native(c, async (ctx) => json(await svc.listWebsiteAdmins(ctx, c.req.param("websiteKey")))));
app.post("/api/v1/websites/:websiteKey/admins", (c) =>
  native(c, async (ctx) => json(await svc.addWebsiteAdminByInput(ctx, c.req.param("websiteKey"), await bodyJson(c)), 201))
);
app.delete("/api/v1/websites/:websiteKey/admins/:userId", (c) =>
  native(c, async (ctx) => {
    await svc.removeWebsiteAdmin(ctx, c.req.param("websiteKey"), numberParam(c, "userId"));
    return empty();
  })
);

app.put("/api/v1/websites/:websiteKey/pages/:pageKey", (c) =>
  native(c, async (ctx) => json(await svc.upsertPage(ctx, c.req.param("websiteKey"), c.req.param("pageKey"), await bodyJson(c))))
);
app.get("/api/v1/websites/:websiteKey/pages", (c) => native(c, async (ctx) => json(await svc.listPages(ctx, c.req.param("websiteKey"), query(c)))));
app.get("/api/v1/websites/:websiteKey/pages/:pageKey", (c) =>
  native(c, async (ctx) => json(await svc.getPageResponse(ctx, c.req.param("websiteKey"), c.req.param("pageKey"))))
);
app.get("/api/v1/websites/:websiteKey/pages/:pageKey/comments", (c) =>
  native(c, async (ctx) => json(await svc.listPageComments(ctx, c.req.param("websiteKey"), c.req.param("pageKey"), query(c))))
);
app.post("/api/v1/websites/:websiteKey/pages/:pageKey/comments", (c) =>
  native(c, async (ctx) => json(await svc.createPageComment(ctx, c.req.param("websiteKey"), c.req.param("pageKey"), await bodyJson(c)), 201))
);
app.patch("/api/v1/websites/:websiteKey/comments/:commentId", (c) =>
  native(c, async (ctx) => json(await svc.updateComment(ctx, c.req.param("websiteKey"), numberParam(c, "commentId"), await bodyJson(c))))
);
app.delete("/api/v1/websites/:websiteKey/comments/:commentId", (c) =>
  native(c, async (ctx) => {
    await svc.deleteComment(ctx, c.req.param("websiteKey"), numberParam(c, "commentId"));
    return empty();
  })
);
app.put("/api/v1/websites/:websiteKey/comments/:commentId/reactions/:content", (c) =>
  native(c, async (ctx) => json(await svc.setCommentReaction(ctx, c.req.param("websiteKey"), numberParam(c, "commentId"), c.req.param("content"))))
);
app.delete("/api/v1/websites/:websiteKey/comments/:commentId/reactions/:content", (c) =>
  native(c, async (ctx) => {
    await svc.deleteCommentReaction(ctx, c.req.param("websiteKey"), numberParam(c, "commentId"), c.req.param("content"));
    return empty();
  })
);

app.get("/api/v1/comments/current", (c) => native(c, async (ctx) => json(await svc.getCurrentComments(ctx, referer(c), query(c)))));
app.post("/api/v1/comments/current", (c) => native(c, async (ctx) => json(await svc.createCurrentComment(ctx, referer(c), await bodyJson(c)), 201)));
app.get("/api/v1/comments/current/replies", (c) => native(c, async (ctx) => json(await svc.listCurrentReplies(ctx, referer(c), query(c)))));
app.put("/api/v1/comments/current/:commentId/reactions/:content", (c) =>
  native(c, async (ctx) => json(await svc.setCurrentReaction(ctx, referer(c), numberParam(c, "commentId"), c.req.param("content"))))
);
app.delete("/api/v1/comments/current/:commentId/reactions/:content", (c) =>
  native(c, async (ctx) => {
    await svc.deleteCurrentReaction(ctx, referer(c), numberParam(c, "commentId"), c.req.param("content"));
    return empty();
  })
);

app.get("/api/v1/websites/:websiteKey/admin/comments", (c) =>
  native(c, async (ctx) => json(await svc.listModerationComments(ctx, c.req.param("websiteKey"), query(c))))
);
app.post("/api/v1/websites/:websiteKey/bans", (c) =>
  native(c, async (ctx) => json(await svc.banWebsiteUser(ctx, c.req.param("websiteKey"), await bodyJson(c)), 201))
);
app.get("/api/v1/websites/:websiteKey/bans", (c) => native(c, async (ctx) => json(await svc.listWebsiteBans(ctx, c.req.param("websiteKey")))));
app.delete("/api/v1/websites/:websiteKey/bans/:userId", (c) =>
  native(c, async (ctx) => {
    await svc.unbanWebsiteUser(ctx, c.req.param("websiteKey"), numberParam(c, "userId"));
    return empty();
  })
);

app.notFound(() => json({ error: "not_found", message: "Not found" }, 404));

async function buildContext(c: Context<{ Bindings: Env; Variables: Vars }>): Promise<AppContext> {
  const baseUrl = c.env.BASE_URL || "http://127.0.0.1:8787";
  const ctx: AppContext = {
    db: new Database(c.env.DB),
    env: c.env,
    baseUrl,
    jwtSecret: parseSecret(c.env.JWT_SECRET),
    statefulSessions: false
  };
  const authHeader = c.req.header("Authorization");
  const bypass = tryTestBypass(authHeader, c.env.ATRIUM_TEST_BYPASS_SECRET || c.env.XTALK_TEST_BYPASS_SECRET);
  if (bypass) {
    await svc.upsertAuthUser(ctx, bypass);
    ctx.user = bypass;
    return ctx;
  }
  const path = new URL(c.req.url).pathname;
  if (path.startsWith("/api/v1/auth/refresh") || path.endsWith("/authorize")) return ctx;
  if (path.startsWith("/api/v1/")) {
    ctx.user = (await resolveNativeRequestUser(ctx, authHeader, c.req.header("Cookie"))) ?? undefined;
  }
  return ctx;
}

async function native(c: Context<{ Bindings: Env; Variables: Vars }>, fn: (ctx: AppContext) => Promise<Response> | Response): Promise<Response> {
  try {
    return await fn(c.get("ctx"));
  } catch (error) {
    const apiError = asApiError(error);
    return json(apiError.nativeBody(), apiError.status);
  }
}

function query(c: Context): URLSearchParams {
  return new URL(c.req.url).searchParams;
}

function referer(c: Context): string | undefined {
  return c.req.header("Referer") ?? c.req.header("Referrer");
}

async function bodyJson(c: Context): Promise<any> {
  try {
    return await c.req.json();
  } catch {
    throw ApiError.badRequest("Invalid request body");
  }
}

function numberParam(c: Context, name: string): number {
  const raw = c.req.param(name);
  if (raw == null) throw ApiError.badRequest(`missing route param: ${name}`);
  const value = Number.parseInt(raw, 10);
  if (!Number.isFinite(value)) throw ApiError.badRequest(`invalid integer param: ${name}`);
  return value;
}

function json(payload: unknown, status = 200): Response {
  return new Response(JSON.stringify(payload), { status, headers: { "Content-Type": "application/json" } });
}

function textResponse(payload: string, status = 200, headers?: HeadersInit): Response {
  const h = new Headers(headers);
  if (!h.has("Content-Type")) h.set("Content-Type", "text/plain; charset=utf-8");
  return new Response(payload, { status, headers: h });
}

function htmlResponse(payload: string, status = 200): Response {
  return new Response(payload, { status, headers: { "Content-Type": "text/html; charset=utf-8" } });
}

function empty(): Response {
  return new Response(null, { status: 204 });
}

function withAuthCookies(ctx: AppContext, response: Response, tokens: { access_token: string; refresh_token: string; expires_in: number }): Response {
  const secure = secureFromBaseUrl(ctx.baseUrl);
  const out = new Response(response.body, response);
  out.headers.append("Set-Cookie", buildSetCookie(ACCESS_COOKIE, tokens.access_token, tokens.expires_in, secure));
  out.headers.append("Set-Cookie", buildSetCookie(REFRESH_COOKIE, tokens.refresh_token, 30 * 24 * 3600, secure));
  return out;
}

async function accountAuthorize(c: Context<{ Bindings: Env; Variables: Vars }>, ctx: AppContext): Promise<Response> {
  const redirectUri = query(c).get("redirect_uri");
  if (!redirectUri) throw ApiError.badRequest("missing redirect_uri query parameter");
  const callback = new URL(`${ctx.baseUrl}/api/v1/auth/account/callback`);
  callback.searchParams.set("redirect_uri", redirectUri);
  const userState = query(c).get("state");
  if (userState) callback.searchParams.set("state", userState);
  const location = accountLoginLocation(ctx.env, callback.toString());
  return new Response(null, { status: 302, headers: { Location: location } });
}

async function accountCallback(c: Context<{ Bindings: Env; Variables: Vars }>, ctx: AppContext): Promise<Response> {
  if (!ctx.user) throw ApiError.unauthorized();
  const redirectUri = query(c).get("redirect_uri") || ctx.baseUrl;
  let response = new Response(null, { status: 302, headers: { Location: redirectWithUserState(redirectUri, query(c).get("state") ?? "") } });
  if (hasUsableJwtSecret(ctx)) {
    response = withAuthCookies(ctx, response, await svc.issueAtriumTokens(ctx, ctx.user));
  }
  return response;
}

async function resolveNativeRequestUser(ctx: AppContext, authHeader: string | undefined, cookieHeader: string | null | undefined): Promise<AuthUser | null> {
  const token = parseToken(authHeader) ?? cookieValue(cookieHeader, ACCESS_COOKIE);
  if (token) {
    ensureJwtSecret(ctx);
    return await svc.resolveAtriumJwtUser(ctx, token);
  }
  return await svc.resolveAccountCookieUser(ctx, cookieHeader);
}

function hasUsableJwtSecret(ctx: AppContext): boolean {
  return ctx.jwtSecret.length >= 16;
}

function ensureJwtSecret(ctx: AppContext): void {
  if (!hasUsableJwtSecret(ctx)) throw ApiError.internal("JWT_SECRET is not configured");
}

function tryTestBypass(authHeader: string | undefined, secret: string | undefined): AuthUser | null {
  if (!authHeader || !secret) return null;
  const rest = authHeader.startsWith("testuser ") ? authHeader.slice(9) : null;
  if (!rest) return null;
  const [given, idText, login, email = "", accountSub = ""] = rest.split(":");
  if (given !== secret) return null;
  const id = Number.parseInt(idText ?? "", 10);
  if (!Number.isFinite(id) || !login) return null;
  return {
    id,
    login,
    email,
    avatar_url: `https://account.jihuayu.com/avatar/${id}`,
    type: "User",
    ...(accountSub ? { account_sub: accountSub } : {})
  };
}

function addCors(c: Context, response: Response): Response {
  const out = new Response(response.body, response);
  const origin = c.req.header("Origin");
  if (origin) {
    out.headers.set("Access-Control-Allow-Origin", origin);
    out.headers.set("Access-Control-Allow-Credentials", "true");
    appendVary(out.headers, "Origin");
  } else {
    out.headers.set("Access-Control-Allow-Origin", "*");
  }
  out.headers.set("Access-Control-Allow-Methods", "GET, POST, PUT, PATCH, DELETE, OPTIONS");
  out.headers.set("Access-Control-Allow-Headers", "Authorization, Content-Type, Accept");
  out.headers.set("Access-Control-Max-Age", "600");
  return out;
}

function appendVary(headers: Headers, value: string): void {
  const current = headers.get("Vary");
  if (!current) {
    headers.set("Vary", value);
    return;
  }
  const values = current.split(",").map((item) => item.trim().toLowerCase());
  if (!values.includes(value.toLowerCase())) headers.set("Vary", `${current}, ${value}`);
}

export default app;
