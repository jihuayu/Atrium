import { introspectAccountCookie } from "./account-auth";
import { ApiError } from "./error";
import type { AppContext, AuthUser, PageRow, ReactionCounts, WebsiteRow } from "./types";
import { EMPTY_REACTION_COUNTS } from "./types";
import {
  decodeCursor,
  encodeCursor,
  parseReactionCounts,
  renderMarkdown,
  sha256Hex,
  signJwt,
  timestampSeconds,
  toIso,
  toPublicUser,
  verifyAtriumJwt
} from "./utils";

const ALLOWED_REACTIONS = new Set(["like", "dislike", "heart", "laugh", "hooray", "confused", "rocket", "eyes"]);

interface UserRow {
  id: number;
  login: string;
  email: string;
  avatar_url: string;
  type: string;
  site_admin: number;
}

interface CommentRow {
  id: number;
  website_id: number;
  website_key: string;
  page_id: number;
  page_key: string;
  parent_comment_id: number | null;
  body: string;
  user_id: number;
  created_at: string;
  updated_at: string;
  deleted_at: string | null;
  reactions: string;
  login: string;
  email: string;
  avatar_url: string;
  user_type: string;
}

interface RefererResolution {
  website: WebsiteRow;
  page: PageRow;
}

function userFromRow(row: UserRow, accountSub?: string): AuthUser {
  return {
    id: row.id,
    login: row.login,
    email: row.email ?? "",
    avatar_url: row.avatar_url ?? "",
    type: row.type ?? "User",
    ...(accountSub ? { account_sub: accountSub } : {})
  };
}

export async function upsertAuthUser(ctx: AppContext, user: AuthUser): Promise<void> {
  await ctx.db.execute(
    "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, 0, datetime('now')) ON CONFLICT(id) DO UPDATE SET login = excluded.login, email = excluded.email, avatar_url = excluded.avatar_url, type = excluded.type, cached_at = datetime('now')",
    [user.id, user.login, user.email, user.avatar_url, user.type]
  );
  if (user.account_sub) {
    await ctx.db.execute(
      "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES (?1, 'account', ?2, ?3, ?4, datetime('now')) ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
      [user.id, user.account_sub, user.email, user.avatar_url]
    );
  }
}

export async function resolveAtriumJwtUser(ctx: AppContext, token: string): Promise<AuthUser> {
  const claims = await verifyAtriumJwt<{ sub?: string; token_type?: string }>(token, ctx.jwtSecret);
  if (claims.token_type === "refresh") throw ApiError.unauthorized();
  const userId = Number.parseInt(String(claims.sub ?? ""), 10);
  if (!Number.isFinite(userId)) throw ApiError.unauthorized();
  const row = await ctx.db.first<UserRow>("SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1", [userId]);
  if (!row) throw ApiError.unauthorized();
  return userFromRow(row);
}

export async function issueAtriumTokens(ctx: AppContext, user: AuthUser) {
  const now = timestampSeconds();
  const accessClaims = {
    sub: String(user.id),
    login: user.login,
    iss: "atrium",
    iat: now,
    exp: now + 3600,
    jti: `acc-${user.id}-${now}`,
    token_type: "access"
  };
  const refreshClaims = {
    sub: String(user.id),
    login: user.login,
    iss: "atrium",
    iat: now,
    exp: now + 30 * 24 * 3600,
    jti: `ref-${user.id}-${now}`,
    token_type: "refresh"
  };
  return {
    access_token: await signJwt(accessClaims, ctx.jwtSecret),
    refresh_token: await signJwt(refreshClaims, ctx.jwtSecret),
    expires_in: 3600,
    token_type: "Bearer",
    user: toPublicUser(user, true)
  };
}

export async function refreshAtriumTokens(ctx: AppContext, refreshToken: string) {
  const claims = await verifyAtriumJwt<{ sub?: string; token_type?: string }>(refreshToken, ctx.jwtSecret);
  if (claims.token_type !== "refresh") throw ApiError.unauthorized();
  const userId = Number.parseInt(String(claims.sub ?? ""), 10);
  if (!Number.isFinite(userId)) throw ApiError.unauthorized();
  const row = await ctx.db.first<UserRow>("SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1", [userId]);
  if (!row) throw ApiError.unauthorized();
  return issueAtriumTokens(ctx, userFromRow(row));
}

export async function resolveAccountCookieUser(ctx: AppContext, cookieHeader: string | null | undefined): Promise<AuthUser | null> {
  const accountUser = await introspectAccountCookie(ctx, cookieHeader);
  if (!accountUser) return null;
  const user = await resolveOrCreateProviderUser(ctx, {
    provider: "account",
    provider_user_id: accountUser.sub,
    login: accountUser.handle || accountUser.email || accountUser.displayName || `account-${accountUser.sub}`,
    email: accountUser.email ?? "",
    avatar_url: accountUser.avatarUrl ?? "",
    type: "User"
  });
  return { ...user, account_sub: accountUser.sub };
}

async function resolveOrCreateProviderUser(
  ctx: AppContext,
  providerUser: {
    provider: string;
    provider_user_id: string;
    login: string;
    email: string;
    avatar_url: string;
    type: string;
  }
): Promise<AuthUser> {
  const identity = await ctx.db.first<UserRow>(
    "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin FROM user_identities ui JOIN users u ON u.id = ui.user_id WHERE ui.provider = ?1 AND ui.provider_user_id = ?2",
    [providerUser.provider, providerUser.provider_user_id]
  );
  if (identity) return userFromRow(identity, providerUser.provider === "account" ? providerUser.provider_user_id : undefined);

  let userId: number | null = null;
  if (providerUser.email && !providerUser.email.endsWith("privaterelay.appleid.com")) {
    const byEmail = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE email = ?1", [providerUser.email]);
    userId = byEmail?.id ?? null;
  }
  if (userId === null) {
    const login = await allocateLogin(ctx, providerUser.login);
    await ctx.db.execute(
      "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, 0, datetime('now'))",
      [login, providerUser.email, providerUser.avatar_url, providerUser.type]
    );
    const row = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE login = ?1", [login]);
    if (!row) throw ApiError.internal("failed to create user");
    userId = row.id;
  }
  await ctx.db.execute(
    "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now')) ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
    [userId, providerUser.provider, providerUser.provider_user_id, providerUser.email, providerUser.avatar_url]
  );
  const row = await ctx.db.first<UserRow>("SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1", [userId]);
  if (!row) throw ApiError.internal("failed to load user");
  return userFromRow(row, providerUser.provider === "account" ? providerUser.provider_user_id : undefined);
}

async function allocateLogin(ctx: AppContext, preferred: string): Promise<string> {
  const base = slugify(preferred) || "user";
  for (let i = 0; i < 1000; i += 1) {
    const candidate = i === 0 ? base : `${base}-${i}`;
    const exists = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE login = ?1", [candidate]);
    if (!exists) return candidate;
  }
  throw ApiError.internal("unable to allocate login");
}

export function requireUser(ctx: AppContext): AuthUser {
  if (!ctx.user) throw ApiError.unauthorized();
  return ctx.user;
}

export async function isSuperAdmin(ctx: AppContext): Promise<boolean> {
  const actor = ctx.user;
  if (!actor) return false;
  const ids = superAdminAccountIds(ctx);
  if (ids.length === 0) return false;
  const lowerIds = ids.map((value) => value.toLowerCase());
  if (actor.account_sub && ids.includes(actor.account_sub)) return true;
  if (actor.email && lowerIds.includes(actor.email.toLowerCase())) return true;
  const identities = await ctx.db.all<{ provider_user_id: string; identity_email: string; user_email: string }>(
    "SELECT ui.provider_user_id, ui.email AS identity_email, u.email AS user_email FROM user_identities ui JOIN users u ON u.id = ui.user_id WHERE ui.user_id = ?1 AND ui.provider = 'account'",
    [actor.id]
  );
  return identities.some((identity) => {
    return (
      ids.includes(identity.provider_user_id) ||
      lowerIds.includes((identity.identity_email ?? "").toLowerCase()) ||
      lowerIds.includes((identity.user_email ?? "").toLowerCase())
    );
  });
}

export async function requireSuperAdmin(ctx: AppContext): Promise<AuthUser> {
  const actor = requireUser(ctx);
  if (!(await isSuperAdmin(ctx))) throw ApiError.forbidden("Super admin required");
  return actor;
}

export async function requireWebsiteAdminOrSuperAdmin(ctx: AppContext, websiteKey: string): Promise<WebsiteRow> {
  const actor = requireUser(ctx);
  const website = await getWebsite(ctx, websiteKey);
  if (await isSuperAdmin(ctx)) return website;
  const hit = await ctx.db.first<{ hit: number }>(
    "SELECT 1 AS hit FROM website_admins WHERE website_id = ?1 AND user_id = ?2 LIMIT 1",
    [website.id, actor.id]
  );
  if (!hit) throw ApiError.forbidden("Website admin required");
  return website;
}

export async function requireNotWebsiteBanned(ctx: AppContext, websiteId: number): Promise<void> {
  const actor = requireUser(ctx);
  const banned = await ctx.db.first<{ hit: number }>(
    "SELECT 1 AS hit FROM website_bans WHERE website_id = ?1 AND user_id = ?2 AND unbanned_at IS NULL LIMIT 1",
    [websiteId, actor.id]
  );
  if (banned) throw ApiError.forbidden("User is disabled for this website");
}

function superAdminAccountIds(ctx: AppContext): string[] {
  return (ctx.env.ATRIUM_SUPER_ADMIN_ACCOUNT_IDS ?? "")
    .split(",")
    .map((value) => value.trim())
    .filter(Boolean);
}

export async function createWebsite(ctx: AppContext, input: any) {
  const actor = await requireSuperAdmin(ctx);
  const key = normalizeKey(input.key, "Website", "key");
  const name = String(input.name ?? key).trim();
  if (!name) throw ApiError.validation("Website", "name", "missing_field");
  const existing = await findWebsite(ctx, key);
  if (existing) throw new ApiError(409, "Website already exists");
  await ctx.db.execute("INSERT INTO websites (key, name, created_at, updated_at) VALUES (?1, ?2, datetime('now'), datetime('now'))", [key, name]);
  const website = await getWebsite(ctx, key);
  await replaceWebsiteOrigins(ctx, website.id, input.origins);
  await addWebsiteAdmin(ctx, website.id, actor.id);
  if (Array.isArray(input.admin_user_ids)) {
    for (const userId of input.admin_user_ids) await addWebsiteAdmin(ctx, website.id, Number(userId));
  }
  return websiteResponse(ctx, await getWebsite(ctx, key));
}

export async function listWebsites(ctx: AppContext, query: URLSearchParams) {
  const actor = requireUser(ctx);
  const limit = listLimit(query);
  const cursorId = query.get("cursor") ? decodeCursor(query.get("cursor")!) : null;
  const params: any[] = [];
  let where = "1 = 1";
  if (cursorId != null) {
    where += " AND w.id > ?1";
    params.push(cursorId);
  }
  if (!(await isSuperAdmin(ctx))) {
    where += ` AND EXISTS (SELECT 1 FROM website_admins wa WHERE wa.website_id = w.id AND wa.user_id = ?${params.length + 1})`;
    params.push(actor.id);
  }
  const rows = await ctx.db.all<WebsiteRow>(
    `SELECT w.id, w.key, w.name, w.created_at, w.updated_at FROM websites w WHERE ${where} ORDER BY w.id ASC LIMIT ?${params.length + 1}`,
    [...params, limit + 1]
  );
  const hasMore = rows.length > limit;
  if (hasMore) rows.pop();
  const data = [];
  for (const row of rows) data.push(await websiteResponse(ctx, row));
  return cursorPage(data, hasMore, rows.at(-1)?.id ?? null);
}

export async function getWebsiteResponse(ctx: AppContext, websiteKey: string) {
  return websiteResponse(ctx, await getWebsite(ctx, websiteKey));
}

export async function updateWebsite(ctx: AppContext, websiteKey: string, input: any) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const sets: string[] = [];
  const params: any[] = [];
  let idx = 1;
  if ("name" in input) {
    const name = String(input.name ?? "").trim();
    if (!name) throw ApiError.validation("Website", "name", "missing_field");
    sets.push(`name = ?${idx++}`);
    params.push(name);
  }
  if (sets.length > 0) {
    sets.push("updated_at = datetime('now')");
    await ctx.db.execute(`UPDATE websites SET ${sets.join(", ")} WHERE id = ?${idx}`, [...params, website.id]);
  }
  if ("origins" in input) await replaceWebsiteOrigins(ctx, website.id, input.origins);
  return websiteResponse(ctx, await getWebsite(ctx, websiteKey));
}

export async function listWebsiteAdmins(ctx: AppContext, websiteKey: string) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const rows = await ctx.db.all<any>(
    "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin, wa.created_at FROM website_admins wa JOIN users u ON u.id = wa.user_id WHERE wa.website_id = ?1 ORDER BY wa.created_at ASC, u.id ASC",
    [website.id]
  );
  return {
    data: rows.map((row) => ({
      user: toPublicUser(userFromRow(row), true),
      created_at: toIso(row.created_at)
    }))
  };
}

export async function addWebsiteAdminByInput(ctx: AppContext, websiteKey: string, input: any) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const userId = Number(input.user_id);
  if (!Number.isFinite(userId)) throw ApiError.validation("WebsiteAdmin", "user_id", "invalid");
  await addWebsiteAdmin(ctx, website.id, userId);
  return listWebsiteAdmins(ctx, websiteKey);
}

export async function removeWebsiteAdmin(ctx: AppContext, websiteKey: string, userId: number): Promise<void> {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const total = (await ctx.db.first<{ total: number }>("SELECT COUNT(*) AS total FROM website_admins WHERE website_id = ?1", [website.id]))?.total ?? 0;
  if (total <= 1 && !(await isSuperAdmin(ctx))) throw ApiError.forbidden("Cannot remove the last website admin");
  await ctx.db.execute("DELETE FROM website_admins WHERE website_id = ?1 AND user_id = ?2", [website.id, userId]);
}

async function addWebsiteAdmin(ctx: AppContext, websiteId: number, userId: number): Promise<void> {
  const user = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE id = ?1", [userId]);
  if (!user) throw ApiError.validation("WebsiteAdmin", "user_id", "invalid");
  await ctx.db.execute(
    "INSERT INTO website_admins (website_id, user_id, created_at) VALUES (?1, ?2, datetime('now')) ON CONFLICT(website_id, user_id) DO NOTHING",
    [websiteId, userId]
  );
}

async function replaceWebsiteOrigins(ctx: AppContext, websiteId: number, rawOrigins: unknown): Promise<void> {
  if (rawOrigins == null) return;
  if (!Array.isArray(rawOrigins)) throw ApiError.validation("Website", "origins", "invalid");
  await ctx.db.execute("DELETE FROM website_origins WHERE website_id = ?1", [websiteId]);
  const seen = new Set<string>();
  for (const rawOrigin of rawOrigins) {
    const origin = normalizeOrigin(String(rawOrigin ?? ""));
    if (!origin || seen.has(origin)) continue;
    seen.add(origin);
    await ctx.db.execute("INSERT INTO website_origins (website_id, origin, created_at) VALUES (?1, ?2, datetime('now'))", [websiteId, origin]);
  }
}

export async function upsertPage(ctx: AppContext, websiteKey: string, pageKey: string, input: any) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  return upsertPageForWebsite(ctx, website, pageKey, input);
}

export async function getPageResponse(ctx: AppContext, websiteKey: string, pageKey: string) {
  const website = await getWebsite(ctx, websiteKey);
  return pageResponse(await getPage(ctx, website.id, pageKey), website);
}

export async function listPages(ctx: AppContext, websiteKey: string, query: URLSearchParams) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const limit = listLimit(query);
  const cursorId = query.get("cursor") ? decodeCursor(query.get("cursor")!) : null;
  const params: any[] = [website.id];
  let where = "website_id = ?1";
  if (cursorId != null) {
    where += " AND id > ?2";
    params.push(cursorId);
  }
  const rows = await ctx.db.all<PageRow>(
    `SELECT id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at FROM pages WHERE ${where} ORDER BY id ASC LIMIT ?${params.length + 1}`,
    [...params, limit + 1]
  );
  const hasMore = rows.length > limit;
  if (hasMore) rows.pop();
  return cursorPage(rows.map((row) => pageResponse(row, website)), hasMore, rows.at(-1)?.id ?? null);
}

async function upsertPageForWebsite(ctx: AppContext, website: WebsiteRow, pageKey: string, input: any) {
  const key = normalizeKey(pageKey, "Page", "key");
  const rawUrl = String(input.url ?? "").trim();
  if (!rawUrl) throw ApiError.validation("Page", "url", "missing_field");
  const normalizedUrl = normalizePageUrl(rawUrl);
  const title = String(input.title ?? normalizedUrl).trim();
  if (!title) throw ApiError.validation("Page", "title", "missing_field");
  const metadata = input.metadata == null ? null : JSON.stringify(input.metadata);
  await ctx.db.execute(
    "INSERT INTO pages (website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, datetime('now'), datetime('now')) ON CONFLICT(website_id, key) DO UPDATE SET title = excluded.title, url = excluded.url, normalized_url = excluded.normalized_url, metadata = excluded.metadata, updated_at = datetime('now')",
    [website.id, key, title, rawUrl, normalizedUrl, metadata]
  );
  return pageResponse(await getPage(ctx, website.id, key), website);
}

export async function createPageComment(ctx: AppContext, websiteKey: string, pageKey: string, input: any) {
  const actor = requireUser(ctx);
  const website = await getWebsite(ctx, websiteKey);
  await requireNotWebsiteBanned(ctx, website.id);
  const page = await getPage(ctx, website.id, pageKey);
  const body = String(input.body ?? "");
  if (!body.trim()) throw ApiError.validation("Comment", "body", "missing_field");
  const parentId = input.parent_id == null ? null : Number(input.parent_id);
  if (parentId != null) await ensureActiveCommentOnPage(ctx, website.id, page.id, parentId);
  const row = await ctx.db.first<{ id: number }>(
    "INSERT INTO comments (website_id, page_id, parent_comment_id, body, user_id, reactions, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, '{}', datetime('now'), datetime('now')) RETURNING id",
    [website.id, page.id, parentId, body, actor.id]
  );
  if (!row) throw ApiError.internal("comment insert failed");
  await ctx.db.execute("UPDATE pages SET comment_count = comment_count + 1, updated_at = datetime('now') WHERE id = ?1", [page.id]);
  return commentResponse(await getCommentRow(ctx, website.id, row.id));
}

export async function listPageComments(ctx: AppContext, websiteKey: string, pageKey: string, query: URLSearchParams) {
  const website = await getWebsite(ctx, websiteKey);
  const page = await getPage(ctx, website.id, pageKey);
  const parent = query.get("parent_id") ?? "root";
  return listCommentsForPage(ctx, website, page, parent, query);
}

export async function updateComment(ctx: AppContext, websiteKey: string, commentId: number, input: any) {
  const actor = requireUser(ctx);
  const website = await getWebsite(ctx, websiteKey);
  await requireNotWebsiteBanned(ctx, website.id);
  const row = await getCommentRow(ctx, website.id, commentId);
  if (row.deleted_at) throw ApiError.notFound("Comment");
  if (row.user_id !== actor.id) throw ApiError.forbidden("You are not allowed to edit this comment");
  const body = String(input.body ?? "");
  if (!body.trim()) throw ApiError.validation("Comment", "body", "missing_field");
  await ctx.db.execute("UPDATE comments SET body = ?1, updated_at = datetime('now') WHERE id = ?2 AND website_id = ?3", [body, commentId, website.id]);
  return commentResponse(await getCommentRow(ctx, website.id, commentId));
}

export async function deleteComment(ctx: AppContext, websiteKey: string, commentId: number): Promise<void> {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const row = await getCommentRow(ctx, website.id, commentId);
  if (row.deleted_at) return;
  await ctx.db.execute("UPDATE comments SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND website_id = ?2", [commentId, website.id]);
  await ctx.db.execute("UPDATE pages SET comment_count = CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END, updated_at = datetime('now') WHERE id = ?1", [row.page_id]);
}

export async function setCommentReaction(ctx: AppContext, websiteKey: string, commentId: number, content: string): Promise<ReactionCounts> {
  const actor = requireUser(ctx);
  const website = await getWebsite(ctx, websiteKey);
  await requireNotWebsiteBanned(ctx, website.id);
  if (!ALLOWED_REACTIONS.has(content)) throw ApiError.validation("Reaction", "content", "invalid");
  await ensureActiveCommentInWebsite(ctx, website.id, commentId);
  await ctx.db.execute(
    "INSERT INTO comment_reactions (comment_id, user_id, content, created_at) VALUES (?1, ?2, ?3, datetime('now')) ON CONFLICT(comment_id, user_id, content) DO NOTHING",
    [commentId, actor.id, content]
  );
  return rebuildCachedReactions(ctx, commentId);
}

export async function deleteCommentReaction(ctx: AppContext, websiteKey: string, commentId: number, content: string): Promise<void> {
  const actor = requireUser(ctx);
  const website = await getWebsite(ctx, websiteKey);
  await requireNotWebsiteBanned(ctx, website.id);
  if (!ALLOWED_REACTIONS.has(content)) throw ApiError.validation("Reaction", "content", "invalid");
  await ensureActiveCommentInWebsite(ctx, website.id, commentId);
  const affected = await ctx.db.execute(
    "DELETE FROM comment_reactions WHERE comment_id = ?1 AND user_id = ?2 AND content = ?3",
    [commentId, actor.id, content]
  );
  if (affected > 0) await rebuildCachedReactions(ctx, commentId);
}

export async function listModerationComments(ctx: AppContext, websiteKey: string, query: URLSearchParams) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const limit = listLimit(query);
  const cursorId = query.get("cursor") ? decodeCursor(query.get("cursor")!) : null;
  const filters = ["c.website_id = ?1"];
  const params: any[] = [website.id];
  let idx = 2;
  if (cursorId != null) {
    filters.push(`c.id > ?${idx++}`);
    params.push(cursorId);
  }
  const status = query.get("status") ?? "all";
  if (status === "active") filters.push("c.deleted_at IS NULL");
  else if (status === "deleted") filters.push("c.deleted_at IS NOT NULL");
  else if (status !== "all") throw ApiError.badRequest("invalid status");
  const pageKey = query.get("page_key");
  if (pageKey) {
    filters.push(`p.key = ?${idx++}`);
    params.push(pageKey);
  }
  const authorId = query.get("author_id");
  if (authorId) {
    filters.push(`c.user_id = ?${idx++}`);
    params.push(Number(authorId));
  }
  const pointers = await ctx.db.all<{ id: number }>(
    `SELECT c.id FROM comments c JOIN pages p ON p.id = c.page_id WHERE ${filters.join(" AND ")} ORDER BY c.id ASC LIMIT ?${idx}`,
    [...params, limit + 1]
  );
  const hasMore = pointers.length > limit;
  if (hasMore) pointers.pop();
  const data = [];
  for (const pointer of pointers) data.push(commentResponse(await getCommentRow(ctx, website.id, pointer.id)));
  return cursorPage(data, hasMore, pointers.at(-1)?.id ?? null);
}

export async function banWebsiteUser(ctx: AppContext, websiteKey: string, input: any) {
  const actor = requireUser(ctx);
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const userId = Number(input.user_id);
  if (!Number.isFinite(userId)) throw ApiError.validation("WebsiteBan", "user_id", "invalid");
  const user = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE id = ?1", [userId]);
  if (!user) throw ApiError.validation("WebsiteBan", "user_id", "invalid");
  const reason = input.reason == null ? null : String(input.reason);
  await ctx.db.execute(
    "INSERT INTO website_bans (website_id, user_id, reason, banned_by_user_id, banned_at, unbanned_at) VALUES (?1, ?2, ?3, ?4, datetime('now'), NULL) ON CONFLICT(website_id, user_id) DO UPDATE SET reason = excluded.reason, banned_by_user_id = excluded.banned_by_user_id, banned_at = datetime('now'), unbanned_at = NULL",
    [website.id, userId, reason, actor.id]
  );
  return listWebsiteBans(ctx, websiteKey);
}

export async function listWebsiteBans(ctx: AppContext, websiteKey: string) {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  const rows = await ctx.db.all<any>(
    "SELECT u.id, wb.reason, wb.banned_at, u.login, u.email, u.avatar_url, u.type, u.site_admin FROM website_bans wb JOIN users u ON u.id = wb.user_id WHERE wb.website_id = ?1 AND wb.unbanned_at IS NULL ORDER BY wb.banned_at DESC, wb.user_id ASC",
    [website.id]
  );
  return {
    data: rows.map((row) => ({
      user: toPublicUser(userFromRow(row), true),
      reason: row.reason,
      banned_at: toIso(row.banned_at)
    }))
  };
}

export async function unbanWebsiteUser(ctx: AppContext, websiteKey: string, userId: number): Promise<void> {
  const website = await requireWebsiteAdminOrSuperAdmin(ctx, websiteKey);
  await ctx.db.execute(
    "UPDATE website_bans SET unbanned_at = datetime('now') WHERE website_id = ?1 AND user_id = ?2 AND unbanned_at IS NULL",
    [website.id, userId]
  );
}

export async function getCurrentComments(ctx: AppContext, referer: string | undefined, query: URLSearchParams) {
  const resolved = await resolveCurrentPage(ctx, referer, query.get("page_title"));
  const comments = await listCommentsForPage(ctx, resolved.website, resolved.page, "root", query);
  return {
    website: await websiteResponse(ctx, resolved.website),
    page: pageResponse(resolved.page, resolved.website),
    comments
  };
}

export async function createCurrentComment(ctx: AppContext, referer: string | undefined, input: any) {
  const title = typeof input.page_title === "string" ? input.page_title : null;
  const resolved = await resolveCurrentPage(ctx, referer, title);
  return createPageComment(ctx, resolved.website.key, resolved.page.key, input);
}

export async function listCurrentReplies(ctx: AppContext, referer: string | undefined, query: URLSearchParams) {
  const resolved = await resolveCurrentPage(ctx, referer, query.get("page_title"));
  const commentId = query.get("comment_id");
  if (!commentId) throw ApiError.badRequest("missing comment_id");
  return listCommentsForPage(ctx, resolved.website, resolved.page, commentId, query);
}

export async function setCurrentReaction(ctx: AppContext, referer: string | undefined, commentId: number, content: string) {
  const resolved = await resolveCurrentPage(ctx, referer, null);
  await ensureCommentOnPage(ctx, resolved.website.id, resolved.page.id, commentId);
  return setCommentReaction(ctx, resolved.website.key, commentId, content);
}

export async function deleteCurrentReaction(ctx: AppContext, referer: string | undefined, commentId: number, content: string): Promise<void> {
  const resolved = await resolveCurrentPage(ctx, referer, null);
  await ensureCommentOnPage(ctx, resolved.website.id, resolved.page.id, commentId);
  await deleteCommentReaction(ctx, resolved.website.key, commentId, content);
}

async function resolveCurrentPage(ctx: AppContext, referer: string | undefined, title: string | null): Promise<RefererResolution> {
  if (!referer) throw ApiError.badRequest("missing Referer header");
  const { website, normalizedUrl } = await resolveWebsiteByReferer(ctx, referer);
  const pageKey = await pageKeyFromUrl(normalizedUrl);
  await upsertPageForWebsite(ctx, website, pageKey, {
    title: title || normalizedUrl,
    url: normalizedUrl,
    metadata: null
  });
  return { website, page: await getPage(ctx, website.id, pageKey) };
}

export async function resolveWebsiteByReferer(ctx: AppContext, referer: string): Promise<{ website: WebsiteRow; normalizedUrl: string }> {
  const normalizedUrl = normalizePageUrl(referer);
  const origin = normalizeOrigin(normalizedUrl);
  const row = await ctx.db.first<WebsiteRow>(
    "SELECT w.id, w.key, w.name, w.created_at, w.updated_at FROM website_origins wo JOIN websites w ON w.id = wo.website_id WHERE wo.origin = ?1 LIMIT 1",
    [origin]
  );
  if (!row) throw new ApiError(404, "website_not_found");
  return { website: row, normalizedUrl };
}

export function normalizePageUrl(raw: string): string {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw ApiError.validation("Page", "url", "invalid");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw ApiError.validation("Page", "url", "invalid");
  url.hash = "";
  url.hostname = url.hostname.toLowerCase();
  if ((url.protocol === "https:" && url.port === "443") || (url.protocol === "http:" && url.port === "80")) url.port = "";
  const params = [...url.searchParams.entries()].sort(([aKey, aValue], [bKey, bValue]) => {
    const keyCmp = aKey.localeCompare(bKey);
    return keyCmp === 0 ? aValue.localeCompare(bValue) : keyCmp;
  });
  url.search = "";
  for (const [key, value] of params) url.searchParams.append(key, value);
  return url.toString();
}

export async function pageKeyFromUrl(normalizedUrl: string): Promise<string> {
  return `url-${(await sha256Hex(normalizedUrl)).slice(0, 32)}`;
}

function normalizeOrigin(raw: string): string {
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw ApiError.validation("Website", "origins", "invalid");
  }
  if (url.protocol !== "http:" && url.protocol !== "https:") throw ApiError.validation("Website", "origins", "invalid");
  url.hostname = url.hostname.toLowerCase();
  if ((url.protocol === "https:" && url.port === "443") || (url.protocol === "http:" && url.port === "80")) url.port = "";
  return url.origin;
}

async function findWebsite(ctx: AppContext, websiteKey: string): Promise<WebsiteRow | null> {
  return ctx.db.first<WebsiteRow>(
    "SELECT id, key, name, created_at, updated_at FROM websites WHERE key = ?1",
    [websiteKey]
  );
}

async function getWebsite(ctx: AppContext, websiteKey: string): Promise<WebsiteRow> {
  const row = await findWebsite(ctx, websiteKey);
  if (!row) throw ApiError.notFound("Website");
  return row;
}

async function getPage(ctx: AppContext, websiteId: number, pageKey: string): Promise<PageRow> {
  const row = await ctx.db.first<PageRow>(
    "SELECT id, website_id, key, title, url, normalized_url, metadata, comment_count, created_at, updated_at FROM pages WHERE website_id = ?1 AND key = ?2",
    [websiteId, pageKey]
  );
  if (!row) throw ApiError.notFound("Page");
  return row;
}

async function websiteResponse(ctx: AppContext, website: WebsiteRow) {
  const origins = await ctx.db.all<{ origin: string }>("SELECT origin FROM website_origins WHERE website_id = ?1 ORDER BY origin ASC", [website.id]);
  return {
    id: website.id,
    key: website.key,
    name: website.name,
    origins: origins.map((row) => row.origin),
    created_at: toIso(website.created_at),
    updated_at: toIso(website.updated_at)
  };
}

function pageResponse(page: PageRow, website: WebsiteRow) {
  return {
    id: page.id,
    website_key: website.key,
    key: page.key,
    title: page.title,
    url: page.url,
    normalized_url: page.normalized_url,
    metadata: parseJson(page.metadata),
    comment_count: page.comment_count,
    created_at: toIso(page.created_at),
    updated_at: toIso(page.updated_at)
  };
}

async function listCommentsForPage(ctx: AppContext, website: WebsiteRow, page: PageRow, parent: string, query: URLSearchParams) {
  const limit = listLimit(query);
  const order = query.get("order")?.toLowerCase() === "desc" ? "DESC" : "ASC";
  const cursorId = query.get("cursor") ? decodeCursor(query.get("cursor")!) : null;
  const filters = ["c.website_id = ?1", "c.page_id = ?2"];
  const params: any[] = [website.id, page.id];
  let idx = 3;
  if (parent === "root") {
    filters.push("c.parent_comment_id IS NULL");
  } else {
    const parentId = Number(parent);
    if (!Number.isFinite(parentId)) throw ApiError.badRequest("invalid parent_id");
    await ensureCommentOnPage(ctx, website.id, page.id, parentId);
    filters.push(`c.parent_comment_id = ?${idx++}`);
    params.push(parentId);
  }
  if (cursorId != null) {
    filters.push(order === "DESC" ? `c.id < ?${idx++}` : `c.id > ?${idx++}`);
    params.push(cursorId);
  }
  const pointers = await ctx.db.all<{ id: number }>(
    `SELECT c.id FROM comments c WHERE ${filters.join(" AND ")} ORDER BY c.id ${order} LIMIT ?${idx}`,
    [...params, limit + 1]
  );
  const hasMore = pointers.length > limit;
  if (hasMore) pointers.pop();
  const data = [];
  for (const pointer of pointers) data.push(commentResponse(await getCommentRow(ctx, website.id, pointer.id)));
  return cursorPage(data, hasMore, pointers.at(-1)?.id ?? null);
}

async function getCommentRow(ctx: AppContext, websiteId: number, commentId: number): Promise<CommentRow> {
  const row = await ctx.db.first<CommentRow>(
    "SELECT c.id, c.website_id, w.key AS website_key, c.page_id, p.key AS page_key, c.parent_comment_id, c.body, c.user_id, c.created_at, c.updated_at, c.deleted_at, c.reactions, u.login, u.email, u.avatar_url, u.type AS user_type FROM comments c JOIN websites w ON w.id = c.website_id JOIN pages p ON p.id = c.page_id JOIN users u ON u.id = c.user_id WHERE c.website_id = ?1 AND c.id = ?2",
    [websiteId, commentId]
  );
  if (!row) throw ApiError.notFound("Comment");
  return row;
}

async function ensureActiveCommentInWebsite(ctx: AppContext, websiteId: number, commentId: number): Promise<void> {
  const row = await ctx.db.first<{ id: number }>(
    "SELECT id FROM comments WHERE website_id = ?1 AND id = ?2 AND deleted_at IS NULL",
    [websiteId, commentId]
  );
  if (!row) throw ApiError.notFound("Comment");
}

async function ensureCommentOnPage(ctx: AppContext, websiteId: number, pageId: number, commentId: number): Promise<void> {
  const row = await ctx.db.first<{ id: number }>(
    "SELECT id FROM comments WHERE website_id = ?1 AND page_id = ?2 AND id = ?3",
    [websiteId, pageId, commentId]
  );
  if (!row) throw ApiError.notFound("Comment");
}

async function ensureActiveCommentOnPage(ctx: AppContext, websiteId: number, pageId: number, commentId: number): Promise<void> {
  const row = await ctx.db.first<{ id: number }>(
    "SELECT id FROM comments WHERE website_id = ?1 AND page_id = ?2 AND id = ?3 AND deleted_at IS NULL",
    [websiteId, pageId, commentId]
  );
  if (!row) throw ApiError.notFound("Comment");
}

function commentResponse(row: CommentRow) {
  const deleted = row.deleted_at != null;
  const author = userFromRow({
    id: row.user_id,
    login: row.login,
    email: row.email,
    avatar_url: row.avatar_url,
    type: row.user_type,
    site_admin: 0
  });
  return {
    id: row.id,
    website_key: row.website_key,
    page_key: row.page_key,
    parent_id: row.parent_comment_id,
    body: deleted ? "" : row.body,
    body_html: deleted ? "" : renderMarkdown(row.body),
    author: toPublicUser(author),
    reactions: parseReactionCounts(row.reactions),
    deleted,
    created_at: toIso(row.created_at),
    updated_at: toIso(row.updated_at),
    deleted_at: toIso(row.deleted_at)
  };
}

async function rebuildCachedReactions(ctx: AppContext, commentId: number): Promise<ReactionCounts> {
  const row = await ctx.db.first<Omit<ReactionCounts, "total">>(
    "SELECT COALESCE(SUM(CASE WHEN content = 'like' THEN 1 ELSE 0 END), 0) AS like, COALESCE(SUM(CASE WHEN content = 'dislike' THEN 1 ELSE 0 END), 0) AS dislike, COALESCE(SUM(CASE WHEN content = 'heart' THEN 1 ELSE 0 END), 0) AS heart, COALESCE(SUM(CASE WHEN content = 'laugh' THEN 1 ELSE 0 END), 0) AS laugh, COALESCE(SUM(CASE WHEN content = 'hooray' THEN 1 ELSE 0 END), 0) AS hooray, COALESCE(SUM(CASE WHEN content = 'confused' THEN 1 ELSE 0 END), 0) AS confused, COALESCE(SUM(CASE WHEN content = 'rocket' THEN 1 ELSE 0 END), 0) AS rocket, COALESCE(SUM(CASE WHEN content = 'eyes' THEN 1 ELSE 0 END), 0) AS eyes FROM comment_reactions WHERE comment_id = ?1",
    [commentId]
  );
  const counts: ReactionCounts = {
    like: Number(row?.like ?? 0),
    dislike: Number(row?.dislike ?? 0),
    heart: Number(row?.heart ?? 0),
    laugh: Number(row?.laugh ?? 0),
    hooray: Number(row?.hooray ?? 0),
    confused: Number(row?.confused ?? 0),
    rocket: Number(row?.rocket ?? 0),
    eyes: Number(row?.eyes ?? 0),
    total: 0
  };
  counts.total = counts.like + counts.dislike + counts.heart + counts.laugh + counts.hooray + counts.confused + counts.rocket + counts.eyes;
  await ctx.db.execute("UPDATE comments SET reactions = ?1, updated_at = updated_at WHERE id = ?2", [JSON.stringify(counts), commentId]);
  return counts;
}

function cursorPage<T>(data: T[], hasMore: boolean, lastId: number | null) {
  return {
    data,
    pagination: {
      next_cursor: hasMore && lastId != null ? encodeCursor(lastId) : null,
      has_more: hasMore
    }
  };
}

function listLimit(query: URLSearchParams): number {
  return Math.min(100, Math.max(1, Number.parseInt(query.get("limit") ?? "20", 10) || 20));
}

function normalizeKey(value: unknown, resource: string, field: string): string {
  const key = String(value ?? "").trim().toLowerCase();
  if (!/^[a-z0-9][a-z0-9_-]{1,127}$/.test(key)) throw ApiError.validation(resource, field, "invalid");
  return key;
}

function slugify(value: string): string {
  return value.trim().toLowerCase().replace(/[^a-z0-9_-]+/g, "-").replace(/^-+|-+$/g, "").slice(0, 64);
}

function parseJson(raw: string | null): unknown {
  if (!raw) return null;
  try {
    return JSON.parse(raw);
  } catch {
    return null;
  }
}
