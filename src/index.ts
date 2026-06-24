import { Hono } from "hono";
import type { Context, Next } from "hono";
import {
  accountCallbackUri,
  buildAccountAuthorizeLocation,
  exchangeAccountAuthorizationCode,
  redirectWithUserState,
  verifyAccountOAuthState,
  type AccountAuthRoute
} from "./account-auth";
import { Database } from "./db";
import { ApiError, asApiError } from "./error";
import * as svc from "./services";
import type { AppContext, Env, GitHubUser } from "./types";
import {
  ACCESS_COOKIE,
  REFRESH_COOKIE,
  bearerFromHeader,
  buildLinkHeader,
  buildSetCookie,
  clearCookie,
  cookieValue,
  parseSecret,
  parseToken,
  renderMarkdown,
  secureFromBaseUrl
} from "./utils";

type Vars = { ctx: AppContext };
const app = new Hono<{ Bindings: Env; Variables: Vars }>();

app.use("*", async (c, next) => {
  if (c.req.method === "OPTIONS") {
    return addCors(new Response(null, { status: 204, headers: { Allow: "GET,POST,PATCH,DELETE,OPTIONS" } }));
  }
  let ctx: AppContext;
  try {
    ctx = await buildContext(c);
  } catch (error) {
    const apiError = asApiError(error);
    const path = new URL(c.req.url).pathname;
    const body = path.startsWith("/api/v1/") ? apiError.nativeBody() : apiError.githubBody();
    return addCors(json(body, apiError.status));
  }
  c.set("ctx", ctx);
  await next();
  c.res = addCors(c.res);
});

app.get("/", (c) =>
  textResponse(
    `Atrium - GitHub Issues compatible comment backend

GitHub-Compatible API (token auth):
  GET    /repos/{owner}/{repo}/issues
  POST   /repos/{owner}/{repo}/issues
  GET    /repos/{owner}/{repo}/issues/{number}
  PATCH  /repos/{owner}/{repo}/issues/{number}
  GET    /repos/{owner}/{repo}/issues/{number}/comments
  POST   /repos/{owner}/{repo}/issues/{number}/comments
  GET    /repos/{owner}/{repo}/issues/comments/{id}
  PATCH  /repos/{owner}/{repo}/issues/comments/{id}
  DELETE /repos/{owner}/{repo}/issues/comments/{id}
  POST   /repos/{owner}/{repo}/issues/comments/{id}/reactions
  DELETE /repos/{owner}/{repo}/issues/comments/{id}/reactions/{id}
  GET    /search/issues?q=...

Native API (JWT auth):
  POST   /api/v1/auth/account
  GET    /api/v1/auth/account/authorize
  GET    /api/v1/auth/account/callback
  POST   /api/v1/auth/github              (legacy GitHub token exchange)
  GET    /api/v1/auth/github/authorize    (account login compatibility alias)
  GET    /api/v1/auth/github/callback     (account login compatibility alias)
  POST   /api/v1/auth/refresh
  DELETE /api/v1/auth/session
  GET    /api/v1/auth/me
  POST   /api/v1/repos
  GET    /api/v1/repos/{owner}/{repo}/threads
  POST   /api/v1/repos/{owner}/{repo}/threads
  GET    /api/v1/repos/{owner}/{repo}/threads/{number}
  PATCH  /api/v1/repos/{owner}/{repo}/threads/{number}
  DELETE /api/v1/repos/{owner}/{repo}/threads/{number}
  GET    /api/v1/repos/{owner}/{repo}/threads/{number}/comments
  POST   /api/v1/repos/{owner}/{repo}/threads/{number}/comments
  GET    /api/v1/repos/{owner}/{repo}/comments/{id}
  PATCH  /api/v1/repos/{owner}/{repo}/comments/{id}
  DELETE /api/v1/repos/{owner}/{repo}/comments/{id}
  POST   /api/v1/repos/{owner}/{repo}/comments/{id}/reactions
  DELETE /api/v1/repos/{owner}/{repo}/comments/{id}/reactions/{content}
  GET    /api/v1/repos/{owner}/{repo}/labels
  POST   /api/v1/repos/{owner}/{repo}/labels
  DELETE /api/v1/repos/{owner}/{repo}/labels/{name}
  GET    /api/v1/repos/{owner}/{repo}/export

Source: https://github.com/pnnh/atrium
`
  )
);

app.get("/user", (c) => compat(c, async (ctx) => json(svc.requireUser(ctx) && userBody(svc.requireUser(ctx)))));
app.get("/user/export", (c) => compat(c, async (ctx) => json(await svc.exportUserRepos(ctx))));
app.post("/markdown", (c) =>
  compat(c, async () => {
    const body = await bodyJson(c);
    return html(renderMarkdown(String(body.text ?? "")));
  })
);
app.post("/token", proxyUtterancesToken);
app.post("/api/utterances/token", proxyUtterancesToken);

app.get("/repos/:owner/:repo/issues", (c) =>
  compat(c, async (ctx) => {
    const { items, total, page, perPage } = await svc.listIssues(ctx, c.req.param("owner"), c.req.param("repo"), query(c));
    return json(items, 200, linkHeader(ctx, `/repos/${c.req.param("owner")}/${c.req.param("repo")}/issues`, page, perPage, total));
  })
);
app.post("/repos/:owner/:repo/issues", (c) => compat(c, async (ctx) => json(await svc.createIssue(ctx, c.req.param("owner"), c.req.param("repo"), await bodyJson(c)), 201)));
app.get("/repos/:owner/:repo/issues/:number", (c) => compat(c, async (ctx) => json(await svc.getIssue(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number")))));
app.patch("/repos/:owner/:repo/issues/:number", (c) =>
  compat(c, async (ctx) => json(await svc.updateIssue(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), await bodyJson(c))))
);
app.get("/repos/:owner/:repo/issues/:number/comments", (c) =>
  compat(c, async (ctx) => {
    const { items, total, page, perPage } = await svc.listComments(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), query(c));
    return json(items, 200, linkHeader(ctx, `/repos/${c.req.param("owner")}/${c.req.param("repo")}/issues/${numberParam(c, "number")}/comments`, page, perPage, total));
  })
);
app.post("/repos/:owner/:repo/issues/:number/comments", (c) =>
  compat(c, async (ctx) => json(await svc.createComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), await bodyJson(c)), 201))
);
app.get("/repos/:owner/:repo/issues/comments/:id", (c) => compat(c, async (ctx) => json(await svc.getComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id")))));
app.patch("/repos/:owner/:repo/issues/comments/:id", (c) =>
  compat(c, async (ctx) => json(await svc.updateComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), await bodyJson(c))))
);
app.delete("/repos/:owner/:repo/issues/comments/:id", (c) =>
  compat(c, async (ctx) => {
    await svc.deleteComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"));
    return empty();
  })
);
app.get("/repos/:owner/:repo/issues/comments/:id/reactions", (c) =>
  compat(c, async (ctx) => {
    const { items, total, page, perPage } = await svc.listReactions(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), query(c));
    return json(items, 200, linkHeader(ctx, `/repos/${c.req.param("owner")}/${c.req.param("repo")}/issues/comments/${numberParam(c, "id")}/reactions`, page, perPage, total));
  })
);
app.post("/repos/:owner/:repo/issues/comments/:id/reactions", (c) =>
  compat(c, async (ctx) => {
    const result = await svc.createReaction(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), await bodyJson(c));
    return json(result.reaction, result.created ? 201 : 200);
  })
);
app.delete("/repos/:owner/:repo/issues/comments/:id/reactions/:rid", (c) =>
  compat(c, async (ctx) => {
    await svc.deleteReaction(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), numberParam(c, "rid"));
    return empty();
  })
);
app.get("/repos/:owner/:repo/labels", (c) => compat(c, async (ctx) => json(await svc.listLabels(ctx, c.req.param("owner"), c.req.param("repo")))));
app.post("/repos/:owner/:repo/labels", (c) => compat(c, async (ctx) => json(await svc.createLabel(ctx, c.req.param("owner"), c.req.param("repo"), await bodyJson(c)), 201)));
app.get("/search/issues", (c) =>
  compat(c, async (ctx) => {
    const { items, total, page, perPage } = await svc.searchIssues(ctx, query(c));
    return json({ total_count: total, incomplete_results: false, items }, 200, linkHeader(ctx, "/search/issues", page, perPage, total));
  })
);

app.post("/api/v1/auth/account", (c) =>
  native(c, async (ctx) => {
    const input = await bodyJson(c);
    const idToken = String(input.id_token ?? input.token ?? "");
    if (!idToken) throw ApiError.badRequest("missing id_token");
    const tokens = await svc.resolveAccountLogin(ctx, idToken);
    return withAuthCookies(ctx, json(tokens), tokens);
  })
);
app.get("/api/v1/auth/account/authorize", (c) => native(c, async (ctx) => accountAuthorize(c, ctx, "account")));
app.get("/api/v1/auth/account/callback", (c) => native(c, async (ctx) => accountCallback(c, ctx, "account")));
app.post("/api/v1/auth/github", (c) =>
  native(c, async (ctx) => {
    const input = await bodyJson(c);
    const user = await svc.resolveGitHubUser(ctx, String(input.token ?? ""));
    const tokens = await svc.issueAtriumTokens(ctx, user);
    return withAuthCookies(ctx, json(tokens), tokens);
  })
);
app.get("/api/v1/auth/github/authorize", (c) => native(c, async (ctx) => accountAuthorize(c, ctx, "github")));
app.get("/api/v1/auth/github/callback", (c) => native(c, async (ctx) => accountCallback(c, ctx, "github")));
app.post("/api/v1/auth/google", () => legacyProviderDisabled("Google"));
app.post("/api/v1/auth/apple", () => legacyProviderDisabled("Apple"));
app.post("/api/v1/auth/refresh", (c) =>
  native(c, async (ctx) => {
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
    if (!ctx.user) {
      const token = parseToken(c.req.header("Authorization")) ?? cookieValue(c.req.header("Cookie"), ACCESS_COOKIE);
      if (!token) throw ApiError.unauthorized();
      ctx.user = await svc.resolveAtriumJwtUser(ctx, token);
    }
    const response = empty();
    const secure = secureFromBaseUrl(ctx.baseUrl);
    response.headers.append("Set-Cookie", clearCookie(ACCESS_COOKIE, secure));
    response.headers.append("Set-Cookie", clearCookie(REFRESH_COOKIE, secure));
    return response;
  })
);
app.get("/api/v1/auth/me", (c) =>
  native(c, async (ctx) => {
    if (!ctx.user) {
      const token = parseToken(c.req.header("Authorization")) ?? cookieValue(c.req.header("Cookie"), ACCESS_COOKIE);
      if (!token) throw ApiError.unauthorized();
      ctx.user = await svc.resolveAtriumJwtUser(ctx, token);
    }
    return json(userBody(ctx.user));
  })
);

app.post("/api/v1/repos", (c) =>
  native(c, async (ctx) => {
    const input = await bodyJson(c);
    const { repo, created } = await svc.createGlobalRepo(ctx, String(input.name ?? ""));
    return json(repoSettings(repo), created ? 201 : 200);
  })
);
app.get("/api/v1/repos/:owner/:repo", (c) =>
  native(c, async (ctx) => {
    const repo = await ensureRepoAdmin(ctx, c.req.param("owner"), c.req.param("repo"));
    return json(repoSettings(repo));
  })
);
app.patch("/api/v1/repos/:owner/:repo", (c) =>
  native(c, async (ctx) => {
    const repo = await ensureRepoAdmin(ctx, c.req.param("owner"), c.req.param("repo"));
    const input = await bodyJson(c);
    if (input.admin_user_id != null) {
      const user = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE id = ?1", [Number(input.admin_user_id)]);
      if (!user) throw ApiError.validation("Repository", "admin_user_id", "invalid");
      await ctx.db.execute("UPDATE repos SET admin_user_id = ?1 WHERE id = ?2", [Number(input.admin_user_id), repo.id]);
    }
    return json(repoSettings(await svc.getRepo(ctx, c.req.param("owner"), c.req.param("repo"))));
  })
);
app.get("/api/v1/repos/:owner/:repo/threads", (c) => native(c, async (ctx) => json(await svc.nativeListThreads(ctx, c.req.param("owner"), c.req.param("repo"), query(c)))));
app.post("/api/v1/repos/:owner/:repo/threads", (c) =>
  native(c, async (ctx) => json(svc.toNativeThread(await svc.createIssue(ctx, c.req.param("owner"), c.req.param("repo"), await bodyJson(c))), 201))
);
app.get("/api/v1/repos/:owner/:repo/threads/:number", (c) =>
  native(c, async (ctx) => json(svc.toNativeThread(await svc.getIssue(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number")))))
);
app.patch("/api/v1/repos/:owner/:repo/threads/:number", (c) =>
  native(c, async (ctx) => json(svc.toNativeThread(await svc.updateIssue(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), await bodyJson(c)))))
);
app.delete("/api/v1/repos/:owner/:repo/threads/:number", (c) =>
  native(c, async (ctx) => {
    await svc.softDeleteIssue(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"));
    return empty();
  })
);
app.get("/api/v1/repos/:owner/:repo/threads/:number/comments", (c) =>
  native(c, async (ctx) => json(await svc.nativeListComments(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), query(c))))
);
app.post("/api/v1/repos/:owner/:repo/threads/:number/comments", (c) =>
  native(c, async (ctx) => json(svc.toNativeComment(await svc.createComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "number"), await bodyJson(c))), 201))
);
app.get("/api/v1/repos/:owner/:repo/comments/:id", (c) => native(c, async (ctx) => json(svc.toNativeComment(await svc.getComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"))))));
app.patch("/api/v1/repos/:owner/:repo/comments/:id", (c) =>
  native(c, async (ctx) => json(svc.toNativeComment(await svc.updateComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), await bodyJson(c)))))
);
app.delete("/api/v1/repos/:owner/:repo/comments/:id", (c) =>
  native(c, async (ctx) => {
    await svc.deleteComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"));
    return empty();
  })
);
app.post("/api/v1/repos/:owner/:repo/comments/:id/reactions", (c) =>
  native(c, async (ctx) => {
    const result = await svc.createReaction(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), await bodyJson(c));
    const comment = await svc.getComment(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"));
    return json(svc.toNativeComment(comment).reactions, result.created ? 201 : 200);
  })
);
app.delete("/api/v1/repos/:owner/:repo/comments/:id/reactions/:content", (c) =>
  native(c, async (ctx) => {
    await svc.deleteReactionByContent(ctx, c.req.param("owner"), c.req.param("repo"), numberParam(c, "id"), c.req.param("content"));
    return empty();
  })
);
app.get("/api/v1/repos/:owner/:repo/labels", (c) =>
  native(c, async (ctx) => json((await svc.listLabels(ctx, c.req.param("owner"), c.req.param("repo"))).map((label) => ({ id: label.id, name: label.name, color: label.color }))))
);
app.post("/api/v1/repos/:owner/:repo/labels", (c) =>
  native(c, async (ctx) => {
    await ensureRepoAdmin(ctx, c.req.param("owner"), c.req.param("repo"));
    const label = await svc.createLabel(ctx, c.req.param("owner"), c.req.param("repo"), await bodyJson(c));
    return json({ id: label.id, name: label.name, color: label.color }, 201);
  })
);
app.delete("/api/v1/repos/:owner/:repo/labels/:name", (c) =>
  native(c, async (ctx) => {
    const repo = await ensureRepoAdmin(ctx, c.req.param("owner"), c.req.param("repo"));
    await ctx.db.execute("DELETE FROM labels WHERE repo_id = ?1 AND name = ?2", [repo.id, c.req.param("name")]);
    return empty();
  })
);
app.get("/api/v1/repos/:owner/:repo/export", (c) =>
  native(c, async (ctx) => {
    const result = await svc.exportNativeRepo(ctx, c.req.param("owner"), c.req.param("repo"), query(c));
    if ("csv" in result) {
      return textResponse(result.csv ?? "", 200, {
        "Content-Type": "text/csv; charset=utf-8",
        "Content-Disposition": `attachment; filename="${c.req.param("owner").replace(/"/g, "")}-${c.req.param("repo").replace(/"/g, "")}-export.csv"`
      });
    }
    return json(result.json);
  })
);

async function buildContext(c: Context<{ Bindings: Env; Variables: Vars }>): Promise<AppContext> {
  const baseUrl = c.env.BASE_URL || "http://127.0.0.1:8787";
  const ctx: AppContext = {
    db: new Database(c.env.DB),
    env: c.env,
    baseUrl,
    tokenCacheTtl: Number.parseInt(c.env.TOKEN_CACHE_TTL ?? "3600", 10) || 3600,
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
  if (path.startsWith("/api/v1/") && ctx.jwtSecret.length < 16) {
    if (!path.startsWith("/api/v1/auth/google") && !path.startsWith("/api/v1/auth/apple")) {
      throw ApiError.internal("JWT_SECRET is not configured");
    }
  }
  if (path.startsWith("/api/v1/auth/")) return ctx;
  if (path.startsWith("/api/v1/")) {
    const token = parseToken(authHeader) ?? cookieValue(c.req.header("Cookie"), ACCESS_COOKIE);
    if (token) ctx.user = await svc.resolveAtriumJwtUser(ctx, token);
    return ctx;
  }
  const token = parseToken(authHeader);
  if (token) ctx.user = await svc.resolveGitHubUser(ctx, token);
  return ctx;
}

async function compat(c: Context<{ Bindings: Env; Variables: Vars }>, fn: (ctx: AppContext) => Promise<Response> | Response): Promise<Response> {
  try {
    return await fn(c.get("ctx"));
  } catch (error) {
    const apiError = asApiError(error);
    return json(apiError.githubBody(), apiError.status);
  }
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

function json(payload: unknown, status = 200, extraHeaders?: Record<string, string | null>): Response {
  const headers = new Headers({ "Content-Type": "application/json" });
  for (const [key, value] of Object.entries(extraHeaders ?? {})) if (value) headers.set(key, value);
  return new Response(JSON.stringify(payload), { status, headers });
}

function html(payload: string): Response {
  return new Response(payload, { status: 200, headers: { "Content-Type": "text/html; charset=utf-8" } });
}

function textResponse(payload: string, status = 200, headers?: HeadersInit): Response {
  const h = new Headers(headers);
  if (!h.has("Content-Type")) h.set("Content-Type", "text/plain; charset=utf-8");
  return new Response(payload, { status, headers: h });
}

function empty(): Response {
  return new Response(null, { status: 204 });
}

function linkHeader(ctx: AppContext, path: string, page: number, perPage: number, total: number) {
  const link = buildLinkHeader(ctx.baseUrl, path, page, perPage, total);
  return link ? { Link: link } : undefined;
}

function userBody(user: GitHubUser) {
  return {
    login: user.login,
    id: user.id,
    avatar_url: user.avatar_url,
    html_url: `https://github.com/${user.login}`,
    type: user.type,
    email: user.email
  };
}

function withAuthCookies(ctx: AppContext, response: Response, tokens: { access_token: string; refresh_token: string; expires_in: number }): Response {
  const secure = secureFromBaseUrl(ctx.baseUrl);
  const out = new Response(response.body, response);
  out.headers.append("Set-Cookie", buildSetCookie(ACCESS_COOKIE, tokens.access_token, tokens.expires_in, secure));
  out.headers.append("Set-Cookie", buildSetCookie(REFRESH_COOKIE, tokens.refresh_token, 30 * 24 * 3600, secure));
  return out;
}

async function accountAuthorize(c: Context<{ Bindings: Env; Variables: Vars }>, ctx: AppContext, route: AccountAuthRoute): Promise<Response> {
  const redirectUri = query(c).get("redirect_uri");
  if (!redirectUri) throw ApiError.badRequest("missing redirect_uri query parameter");
  const location = await buildAccountAuthorizeLocation(ctx, route, redirectUri, query(c).get("state") ?? "");
  return new Response(null, { status: 302, headers: { Location: location } });
}

async function accountCallback(c: Context<{ Bindings: Env; Variables: Vars }>, ctx: AppContext, route: AccountAuthRoute): Promise<Response> {
  const error = query(c).get("error");
  if (error) throw ApiError.badRequest(query(c).get("error_description") ?? error);
  const code = query(c).get("code");
  const stateToken = query(c).get("state");
  if (!code) throw ApiError.badRequest("missing code query parameter");
  if (!stateToken) throw ApiError.badRequest("missing state query parameter");
  const state = await verifyAccountOAuthState(ctx, stateToken);
  const idToken = await exchangeAccountAuthorizationCode(ctx, code, accountCallbackUri(ctx, route));
  const tokens = await svc.resolveAccountLogin(ctx, idToken, state.nonce);
  return withAuthCookies(ctx, new Response(null, { status: 302, headers: { Location: redirectWithUserState(state.redirectUri, state.userState) } }), tokens);
}

function legacyProviderDisabled(provider: string): Response {
  return json({ error: "not_configured", message: `${provider} login has moved to Jihuayu Account` }, 501);
}

async function proxyUtterancesToken(c: Context<{ Bindings: Env; Variables: Vars }>) {
  return compat(c, async () => {
    const headers = new Headers();
    headers.set("Content-Type", c.req.header("Content-Type") ?? "application/json");
    for (const [source, target] of [
      ["Referer", "Referer"],
      ["Origin", "Origin"],
      ["User-Agent", "User-Agent"],
      ["Cookie", "Cookie"],
      ["Sec-CH-UA", "Sec-CH-UA"],
      ["Sec-CH-UA-Mobile", "Sec-CH-UA-Mobile"],
      ["Sec-CH-UA-Platform", "Sec-CH-UA-Platform"]
    ]) {
      const value = c.req.header(source);
      if (value) headers.set(target, value);
    }
    const response = await fetch("https://api.utteranc.es/token", {
      method: "POST",
      headers,
      body: await c.req.text()
    });
    const outHeaders = new Headers();
    for (const name of ["Content-Type", "Cache-Control", "X-Frame-Options", "Content-Security-Policy"]) {
      const value = response.headers.get(name);
      if (value) outHeaders.set(name, value);
    }
    return new Response(await response.arrayBuffer(), { status: response.status, headers: outHeaders });
  });
}

function tryTestBypass(authHeader: string | undefined, secret: string | undefined): GitHubUser | null {
  if (!authHeader || !secret) return null;
  const rest = authHeader.startsWith("testuser ") ? authHeader.slice(9) : null;
  if (!rest) return null;
  const [given, idText, login, email = ""] = rest.split(":");
  if (given !== secret) return null;
  const id = Number.parseInt(idText ?? "", 10);
  if (!Number.isFinite(id) || !login) return null;
  return {
    id,
    login,
    email,
    avatar_url: `https://avatars.githubusercontent.com/u/${id}?v=4`,
    type: "User",
    site_admin: false
  };
}

async function ensureRepoAdmin(ctx: AppContext, owner: string, repoName: string) {
  const actor = svc.requireUser(ctx);
  const repo = await svc.getRepo(ctx, owner, repoName);
  if (repo.admin_user_id !== actor.id) throw ApiError.forbidden("Admin required");
  return repo;
}

function repoSettings(repo: any) {
  return {
    owner: repo.owner,
    name: repo.name,
    owner_user_id: repo.owner_user_id,
    admin_user_id: repo.admin_user_id,
    issue_counter: repo.issue_counter
  };
}

function addCors(response: Response): Response {
  const out = new Response(response.body, response);
  out.headers.set("Access-Control-Allow-Origin", "*");
  out.headers.set("Access-Control-Allow-Methods", "GET, POST, PATCH, DELETE, OPTIONS");
  out.headers.set("Access-Control-Allow-Headers", "Authorization, Content-Type, Accept");
  out.headers.set("Access-Control-Expose-Headers", "Link");
  return out;
}

export default app;
