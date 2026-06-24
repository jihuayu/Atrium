import { ApiError } from "./error";
import { accountClientId, accountIssuer, accountJwksUrl } from "./account-auth";
import type {
  AppContext,
  CommentResponse,
  GitHubUser,
  IssueResponse,
  Label,
  ReactionCounts,
  RepoRow
} from "./types";
import {
  base64Std,
  buildLinkHeader,
  decodeCursor,
  emptyIssueReactions,
  encodeCursor,
  isoNow,
  parseReactionCounts,
  reactionPayload,
  renderMarkdown,
  sha256Hex,
  signJwt,
  timestampSeconds,
  toApiUser,
  toIso,
  verifyAtriumJwt,
  verifyProviderJwt
} from "./utils";

export const GLOBAL_OWNER = "_global";
const ALLOWED_REACTIONS = new Set(["+1", "-1", "laugh", "confused", "heart", "hooray", "rocket", "eyes"]);

interface UserRow {
  id: number;
  login: string;
  email: string;
  avatar_url: string;
  type: string;
  site_admin: number;
}

interface IssueRow {
  id: number;
  number: number;
  title: string;
  body: string | null;
  state: string;
  locked: number;
  user_id: number;
  comment_count: number;
  created_at: string;
  updated_at: string;
  closed_at: string | null;
  login: string;
  avatar_url: string;
  user_type: string;
  site_admin: number;
  repo_id: number;
  repo_owner: string;
  repo_name: string;
  owner_user_id: number | null;
  admin_user_id: number | null;
  slug: string | null;
}

interface CommentRow {
  id: number;
  issue_id: number;
  body: string;
  user_id: number;
  created_at: string;
  updated_at: string;
  reactions: string;
  login: string;
  avatar_url: string;
  user_type: string;
  site_admin: number;
  issue_number: number;
  repo_owner: string;
  repo_name: string;
  admin_user_id: number | null;
}

function userFromRow(row: UserRow): GitHubUser {
  return {
    id: row.id,
    login: row.login,
    email: row.email ?? "",
    avatar_url: row.avatar_url ?? "",
    type: row.type ?? "User",
    site_admin: Number(row.site_admin) !== 0
  };
}

export async function upsertAuthUser(ctx: AppContext, user: GitHubUser): Promise<void> {
  await ctx.db.execute(
    "INSERT INTO users (id, login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, datetime('now')) ON CONFLICT(id) DO UPDATE SET login = excluded.login, email = excluded.email, avatar_url = excluded.avatar_url, type = excluded.type, site_admin = excluded.site_admin, cached_at = datetime('now')",
    [user.id, user.login, user.email, user.avatar_url, user.type, user.site_admin ? 1 : 0]
  );
  await ctx.db.execute(
    "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES (?1, 'github', ?2, ?3, ?4, datetime('now')) ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
    [user.id, String(user.id), user.email, user.avatar_url]
  );
}

export async function resolveGitHubUser(ctx: AppContext, token: string): Promise<GitHubUser> {
  const tokenHash = await sha256Hex(token);
  const cached = await ctx.db.first<UserRow>(
    "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin FROM token_cache tc JOIN users u ON tc.user_id = u.id WHERE tc.token_hash = ?1 AND tc.provider = 'github' AND tc.expires_at > datetime('now')",
    [tokenHash]
  );
  if (cached) return userFromRow(cached);

  const response = await fetch("https://api.github.com/user", {
    headers: {
      Authorization: `Bearer ${token}`,
      Accept: "application/vnd.github+json",
      "User-Agent": "atrium/0.1"
    }
  });
  if (response.status === 401) throw ApiError.unauthorized();
  if (!response.ok) throw new ApiError(response.status, `GitHub API error: ${response.status}`);
  const gh = (await response.json()) as {
    id: number;
    login: string;
    email?: string | null;
    avatar_url: string;
    type: string;
    site_admin: boolean;
  };
  const user = await resolveOrCreateProviderUser(ctx, {
    provider: "github",
    provider_user_id: String(gh.id),
    login: gh.login,
    email: gh.email ?? "",
    avatar_url: gh.avatar_url,
    type: gh.type,
    site_admin: gh.site_admin
  });
  await ctx.db.batch([
    [
      "UPDATE users SET login = ?1, email = ?2, avatar_url = ?3, type = ?4, site_admin = ?5, cached_at = datetime('now') WHERE id = ?6",
      [gh.login, gh.email ?? "", gh.avatar_url, gh.type, gh.site_admin ? 1 : 0, user.id]
    ],
    [
      "INSERT INTO user_identities (user_id, provider, provider_user_id, email, avatar_url, cached_at) VALUES (?1, 'github', ?2, ?3, ?4, datetime('now')) ON CONFLICT(provider, provider_user_id) DO UPDATE SET user_id = excluded.user_id, email = excluded.email, avatar_url = excluded.avatar_url, cached_at = datetime('now')",
      [user.id, String(gh.id), gh.email ?? "", gh.avatar_url]
    ],
    [
      "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) VALUES (?1, 'github', ?2, datetime('now'), datetime('now', '+' || ?3 || ' seconds')) ON CONFLICT(token_hash, provider) DO UPDATE SET user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
      [tokenHash, user.id, ctx.tokenCacheTtl]
    ]
  ]);
  const fresh = await ctx.db.first<UserRow>("SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1", [user.id]);
  if (!fresh) throw ApiError.internal("failed to resolve github user");
  return userFromRow(fresh);
}

export async function resolveAtriumJwtUser(ctx: AppContext, token: string): Promise<GitHubUser> {
  const claims = await verifyAtriumJwt<{ sub?: string; token_type?: string }>(token, ctx.jwtSecret);
  if (claims.token_type === "refresh") throw ApiError.unauthorized();
  const userId = Number.parseInt(String(claims.sub ?? ""), 10);
  if (!Number.isFinite(userId)) throw ApiError.unauthorized();
  const row = await ctx.db.first<UserRow>("SELECT id, login, email, avatar_url, type, site_admin FROM users WHERE id = ?1", [userId]);
  if (!row) throw ApiError.unauthorized();
  return userFromRow(row);
}

export async function issueAtriumTokens(ctx: AppContext, user: GitHubUser) {
  const now = timestampSeconds();
  const accessClaims = {
    sub: String(user.id),
    login: user.login,
    iss: "xtalk",
    iat: now,
    exp: now + 3600,
    jti: `acc-${user.id}-${now}`,
    token_type: "access"
  };
  const refreshClaims = {
    sub: String(user.id),
    login: user.login,
    iss: "xtalk",
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
    user: {
      id: user.id,
      login: user.login,
      avatar_url: user.avatar_url,
      email: user.email
    }
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

export async function resolveProviderLogin(ctx: AppContext, provider: "google" | "apple", token: string, audience: string) {
  const payload = await verifyProviderJwt(
    token,
    provider === "google" ? "https://www.googleapis.com/oauth2/v3/certs" : "https://appleid.apple.com/auth/keys",
    provider === "google" ? "https://accounts.google.com" : "https://appleid.apple.com",
    audience
  );
  const sub = String(payload.sub ?? "");
  if (!sub) throw ApiError.unauthorized();
  const email = typeof payload.email === "string" ? payload.email : "";
  const picture = typeof payload.picture === "string" ? payload.picture : "";
  const user = await resolveOrCreateProviderUser(ctx, {
    provider,
    provider_user_id: sub,
    login: email || `${provider}-${sub}`,
    email,
    avatar_url: picture,
    type: "User",
    site_admin: false
  });
  await cacheProviderToken(ctx, provider, token, user.id, ctx.tokenCacheTtl);
  return issueAtriumTokens(ctx, user);
}

export async function resolveAccountLogin(ctx: AppContext, idToken: string, expectedNonce?: string) {
  const payload = await verifyProviderJwt(idToken, accountJwksUrl(ctx.env), accountIssuer(ctx.env), accountClientId(ctx.env));
  const sub = String(payload.sub ?? "");
  if (!sub) throw ApiError.unauthorized();
  if (expectedNonce !== undefined && String(payload.nonce ?? "") !== expectedNonce) {
    throw ApiError.unauthorized();
  }
  const email = typeof payload.email === "string" ? payload.email : "";
  const picture = typeof payload.picture === "string" ? payload.picture : "";
  const preferredUsername = typeof payload.preferred_username === "string" ? payload.preferred_username : "";
  const name = typeof payload.name === "string" ? payload.name : "";
  const user = await resolveOrCreateProviderUser(ctx, {
    provider: "account",
    provider_user_id: sub,
    login: preferredUsername || email || name || `account-${sub}`,
    email,
    avatar_url: picture,
    type: "User",
    site_admin: false
  });
  await cacheProviderToken(ctx, "account", idToken, user.id, ctx.tokenCacheTtl);
  return issueAtriumTokens(ctx, user);
}

async function cacheProviderToken(ctx: AppContext, provider: string, token: string, userId: number, ttl: number) {
  await ctx.db.execute(
    "INSERT INTO token_cache (token_hash, provider, user_id, cached_at, expires_at) VALUES (?1, ?2, ?3, datetime('now'), datetime('now', '+' || ?4 || ' seconds')) ON CONFLICT(token_hash, provider) DO UPDATE SET user_id = excluded.user_id, cached_at = datetime('now'), expires_at = excluded.expires_at",
    [await sha256Hex(token), provider, userId, ttl]
  );
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
    site_admin: boolean;
  }
): Promise<GitHubUser> {
  const identity = await ctx.db.first<UserRow>(
    "SELECT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin FROM user_identities ui JOIN users u ON u.id = ui.user_id WHERE ui.provider = ?1 AND ui.provider_user_id = ?2",
    [providerUser.provider, providerUser.provider_user_id]
  );
  if (identity) return userFromRow(identity);

  let userId: number | null = null;
  if (providerUser.email && !providerUser.email.endsWith("privaterelay.appleid.com")) {
    const byEmail = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE email = ?1", [providerUser.email]);
    userId = byEmail?.id ?? null;
  }
  if (userId === null) {
    const login = await allocateLogin(ctx, providerUser.login);
    await ctx.db.execute(
      "INSERT INTO users (login, email, avatar_url, type, site_admin, cached_at) VALUES (?1, ?2, ?3, ?4, ?5, datetime('now'))",
      [login, providerUser.email, providerUser.avatar_url, providerUser.type, providerUser.site_admin ? 1 : 0]
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
  return userFromRow(row);
}

async function allocateLogin(ctx: AppContext, preferred: string): Promise<string> {
  const base = preferred.trim() ? preferred.trim().toLowerCase() : "user";
  for (let i = 0; i < 1000; i += 1) {
    const candidate = i === 0 ? base : `${base}-${i}`;
    const exists = await ctx.db.first<{ id: number }>("SELECT id FROM users WHERE login = ?1", [candidate]);
    if (!exists) return candidate;
  }
  throw ApiError.internal("unable to allocate login");
}

export async function findRepo(ctx: AppContext, owner: string, repo: string): Promise<RepoRow | null> {
  return await ctx.db.first<RepoRow>(
    "SELECT r.id, r.owner, r.name, r.owner_user_id, r.admin_user_id, r.issue_counter FROM repos r WHERE r.name = ?2 AND (lower(r.owner) = lower(?1) OR (lower(r.owner) <> lower('_global') AND r.owner_user_id IS NOT NULL AND r.owner_user_id = (SELECT u.id FROM users u JOIN user_identities ui ON ui.user_id = u.id WHERE ui.provider = 'github' AND lower(u.login) = lower(?1) LIMIT 1))) ORDER BY CASE WHEN lower(r.owner) = lower(?1) THEN 0 ELSE 1 END, r.id ASC LIMIT 1",
    [owner, repo]
  );
}

export async function getRepo(ctx: AppContext, owner: string, repo: string): Promise<RepoRow> {
  const row = await findRepo(ctx, owner, repo);
  if (!row) throw ApiError.notFound("Repository");
  return row;
}

export async function ensureRepo(ctx: AppContext, owner: string, repo: string, creator?: GitHubUser): Promise<RepoRow> {
  const existing = await findRepo(ctx, owner, repo);
  if (existing) return existing;
  if (owner.toLowerCase() === GLOBAL_OWNER) throw ApiError.notFound("Repository");
  if (!creator) throw ApiError.notFound("Repository");
  const hasGithub = await ctx.db.first<{ hit: number }>(
    "SELECT 1 AS hit FROM user_identities WHERE user_id = ?1 AND provider = 'github' LIMIT 1",
    [creator.id]
  );
  if (!hasGithub) throw ApiError.forbidden("Repository does not exist. Create one via POST /api/v1/repos first");
  const ownerUser = await ctx.db.first<{ id: number }>(
    "SELECT u.id FROM users u JOIN user_identities ui ON ui.user_id = u.id WHERE ui.provider = 'github' AND lower(u.login) = lower(?1) LIMIT 1",
    [owner]
  );
  const ownerUserId = ownerUser?.id ?? null;
  await ctx.db.execute(
    "INSERT INTO repos (owner, name, owner_user_id, admin_user_id, issue_counter, created_at) VALUES (?1, ?2, ?3, ?4, 0, datetime('now'))",
    [owner, repo, ownerUserId, ownerUserId ?? creator.id]
  );
  const created = await findRepo(ctx, owner, repo);
  if (!created) throw ApiError.internal("failed to create repo");
  return created;
}

export async function createGlobalRepo(ctx: AppContext, name: string): Promise<{ repo: RepoRow; created: boolean }> {
  const actor = requireUser(ctx);
  const repoName = name.trim();
  if (!repoName) throw ApiError.validation("Repository", "name", "missing_field");
  const existing = await findRepo(ctx, GLOBAL_OWNER, repoName);
  if (existing) {
    if (existing.admin_user_id === actor.id || existing.owner_user_id === actor.id) return { repo: existing, created: false };
    throw ApiError.forbidden("Repository already exists");
  }
  await ctx.db.execute(
    "INSERT INTO repos (owner, name, owner_user_id, admin_user_id, issue_counter, created_at) VALUES (?1, ?2, ?3, ?4, 0, datetime('now'))",
    [GLOBAL_OWNER, repoName, actor.id, actor.id]
  );
  const repo = await findRepo(ctx, GLOBAL_OWNER, repoName);
  if (!repo) throw ApiError.internal("failed to create repository");
  return { repo, created: true };
}

export function requireUser(ctx: AppContext): GitHubUser {
  if (!ctx.user) throw ApiError.unauthorized();
  return ctx.user;
}

export async function createIssue(ctx: AppContext, owner: string, repoName: string, input: any): Promise<IssueResponse> {
  const user = requireUser(ctx);
  const title = String(input.title ?? "").trim();
  if (!title) throw ApiError.validation("Issue", "title", "missing_field");
  const repo = await ensureRepo(ctx, owner, repoName, user);
  const counter = await ctx.db.first<{ issue_counter: number }>(
    "UPDATE repos SET issue_counter = issue_counter + 1 WHERE id = ?1 RETURNING issue_counter",
    [repo.id]
  );
  if (!counter) throw ApiError.internal("failed to allocate issue number");
  const slug = typeof input.slug === "string" && input.slug.trim() ? input.slug.trim() : null;
  if (slug) await ensureSlugAvailable(ctx, repo.id, slug);
  await ctx.db.execute(
    "INSERT INTO issues (repo_id, number, title, body, state, locked, user_id, comment_count, created_at, updated_at, slug) VALUES (?1, ?2, ?3, ?4, 'open', 0, ?5, 0, datetime('now'), datetime('now'), ?6)",
    [repo.id, counter.issue_counter, title, input.body ?? null, user.id, slug]
  );
  const issue = await ctx.db.first<{ id: number }>("SELECT id FROM issues WHERE repo_id = ?1 AND number = ?2", [repo.id, counter.issue_counter]);
  if (!issue) throw ApiError.internal("issue insert verification failed");
  if (Array.isArray(input.labels)) await setIssueLabels(ctx, repo.id, issue.id, input.labels);
  return getIssue(ctx, owner, repoName, counter.issue_counter);
}

export async function getIssue(ctx: AppContext, owner: string, repoName: string, number: number): Promise<IssueResponse> {
  const repo = await getRepo(ctx, owner, repoName);
  const row = await fetchIssueRow(ctx, repo.id, number);
  if (!row) throw ApiError.notFound("Issue");
  return buildIssueResponse(ctx, row);
}

export async function listIssues(ctx: AppContext, owner: string, repoName: string, query: URLSearchParams) {
  const repo = await getRepo(ctx, owner, repoName);
  const { page, perPage, offset } = normalizeCompatPagination(query);
  const filters = ["i.repo_id = ?1", "i.deleted_at IS NULL"];
  const params: any[] = [repo.id];
  let idx = 2;
  const state = query.get("state") ?? "open";
  if (state !== "all") {
    filters.push(`i.state = ?${idx++}`);
    params.push(state);
  }
  const creator = query.get("creator");
  if (creator) {
    filters.push(`u.login = ?${idx++}`);
    params.push(creator);
  }
  const since = query.get("since");
  if (since) {
    filters.push(`i.updated_at >= ?${idx++}`);
    params.push(since);
  }
  const labels = query.get("labels");
  if (labels) {
    for (const label of labels.split(",").map((v) => v.trim()).filter(Boolean)) {
      filters.push(`EXISTS (SELECT 1 FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id = i.id AND l.name = ?${idx++})`);
      params.push(label);
    }
  }
  const whereSql = filters.join(" AND ");
  const total = (await ctx.db.first<{ total: number }>(`SELECT COUNT(*) AS total FROM issues i JOIN users u ON u.id = i.user_id WHERE ${whereSql}`, params))?.total ?? 0;
  const sortCol = query.get("sort") === "updated" ? "i.updated_at" : query.get("sort") === "comments" ? "i.comment_count" : "i.created_at";
  const direction = query.get("direction") === "asc" ? "ASC" : "DESC";
  const rows = await ctx.db.all<{ number: number }>(
    `SELECT i.number FROM issues i JOIN users u ON u.id = i.user_id WHERE ${whereSql} ORDER BY ${sortCol} ${direction} LIMIT ?${idx} OFFSET ?${idx + 1}`,
    [...params, perPage, offset]
  );
  const items = [];
  for (const row of rows) items.push(await getIssue(ctx, owner, repoName, row.number));
  return { items, total, page, perPage };
}

export async function updateIssue(ctx: AppContext, owner: string, repoName: string, number: number, input: any): Promise<IssueResponse> {
  const actor = requireUser(ctx);
  const repo = await getRepo(ctx, owner, repoName);
  const row = await fetchIssueRow(ctx, repo.id, number);
  if (!row) throw ApiError.notFound("Issue");
  if (actor.id !== row.user_id && row.admin_user_id !== actor.id) throw ApiError.forbidden("You are not allowed to update this issue");
  const sets: string[] = [];
  const params: any[] = [];
  let idx = 1;
  if ("title" in input) {
    const title = String(input.title ?? "").trim();
    if (!title) throw ApiError.validation("Issue", "title", "missing_field");
    sets.push(`title = ?${idx++}`);
    params.push(title);
  }
  if ("body" in input) {
    sets.push(`body = ?${idx++}`);
    params.push(input.body ?? null);
  }
  if ("state" in input) {
    if (input.state !== "open" && input.state !== "closed") throw ApiError.validation("Issue", "state", "invalid");
    sets.push(`state = ?${idx++}`);
    params.push(input.state);
    sets.push(input.state === "closed" ? "closed_at = datetime('now')" : "closed_at = NULL");
  }
  if ("state_reason" in input) {
    sets.push(`state_reason = ?${idx++}`);
    params.push(input.state_reason ?? null);
  }
  if (sets.length > 0) {
    sets.push("updated_at = datetime('now')");
    await ctx.db.execute(`UPDATE issues SET ${sets.join(", ")} WHERE id = ?${idx}`, [...params, row.id]);
  }
  if (Array.isArray(input.labels)) await setIssueLabels(ctx, row.repo_id, row.id, input.labels);
  return getIssue(ctx, owner, repoName, number);
}

export async function softDeleteIssue(ctx: AppContext, owner: string, repoName: string, number: number): Promise<void> {
  const actor = requireUser(ctx);
  const repo = await getRepo(ctx, owner, repoName);
  if (repo.admin_user_id !== actor.id) throw ApiError.forbidden("Admin required");
  const affected = await ctx.db.execute("UPDATE issues SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE repo_id = ?1 AND number = ?2 AND deleted_at IS NULL", [repo.id, number]);
  if (affected === 0) throw ApiError.notFound("Issue");
}

async function fetchIssueRow(ctx: AppContext, repoId: number, number: number): Promise<IssueRow | null> {
  return ctx.db.first<IssueRow>(
    "SELECT i.id, i.number, i.title, i.body, i.state, i.locked, i.user_id, i.comment_count, i.created_at, i.updated_at, i.closed_at, u.login, u.avatar_url, u.type AS user_type, u.site_admin, r.id AS repo_id, r.owner AS repo_owner, r.name AS repo_name, r.owner_user_id, r.admin_user_id, i.slug FROM issues i JOIN users u ON u.id = i.user_id JOIN repos r ON r.id = i.repo_id WHERE i.repo_id = ?1 AND i.number = ?2 AND i.deleted_at IS NULL",
    [repoId, number]
  );
}

async function ensureSlugAvailable(ctx: AppContext, repoId: number, slug: string) {
  const hit = await ctx.db.first<{ hit: number }>("SELECT 1 AS hit FROM issues WHERE repo_id = ?1 AND slug = ?2 AND deleted_at IS NULL LIMIT 1", [repoId, slug]);
  if (hit) throw new ApiError(409, "Thread slug already exists in this repo");
}

async function setIssueLabels(ctx: AppContext, repoId: number, issueId: number, names: string[]) {
  await ctx.db.execute("DELETE FROM issue_labels WHERE issue_id = ?1", [issueId]);
  const ids = await ensureLabelIds(ctx, repoId, names);
  for (const id of ids) await ctx.db.execute("INSERT OR IGNORE INTO issue_labels (issue_id, label_id) VALUES (?1, ?2)", [issueId, id]);
}

async function issueLabels(ctx: AppContext, issueId: number): Promise<Label[]> {
  return ctx.db.all<Label>(
    "SELECT l.id, l.name, l.color, l.description FROM labels l JOIN issue_labels il ON il.label_id = l.id WHERE il.issue_id = ?1 ORDER BY l.name ASC",
    [issueId]
  );
}

async function buildIssueResponse(ctx: AppContext, row: IssueRow): Promise<IssueResponse> {
  const labels = await issueLabels(ctx, row.id);
  const body = row.body ?? "";
  const user: GitHubUser = {
    id: row.user_id,
    login: row.login,
    email: "",
    avatar_url: row.avatar_url,
    type: row.user_type,
    site_admin: row.site_admin !== 0
  };
  return {
    id: row.id,
    node_id: base64Std(`atrium:Issue:${row.id}`),
    number: row.number,
    title: row.title,
    ...(row.slug ? { slug: row.slug } : {}),
    body,
    body_html: renderMarkdown(body),
    state: row.state,
    locked: row.locked !== 0,
    user: toApiUser(user),
    labels,
    comments: row.comment_count,
    created_at: toIso(row.created_at)!,
    updated_at: toIso(row.updated_at)!,
    closed_at: toIso(row.closed_at),
    author_association: row.admin_user_id === row.user_id ? "OWNER" : "NONE",
    reactions: emptyIssueReactions(ctx.baseUrl, row.repo_owner, row.repo_name, row.number),
    url: `${ctx.baseUrl}/repos/${row.repo_owner}/${row.repo_name}/issues/${row.number}`,
    html_url: `${ctx.baseUrl}/repos/${row.repo_owner}/${row.repo_name}/issues/${row.number}`,
    comments_url: `${ctx.baseUrl}/repos/${row.repo_owner}/${row.repo_name}/issues/${row.number}/comments`
  };
}

function normalizeCompatPagination(query: URLSearchParams) {
  const page = Math.max(1, Number.parseInt(query.get("page") ?? "1", 10) || 1);
  const perPage = Math.min(100, Math.max(1, Number.parseInt(query.get("per_page") ?? "30", 10) || 30));
  return { page, perPage, offset: (page - 1) * perPage };
}

export { buildLinkHeader };

export async function listLabels(ctx: AppContext, owner: string, repoName: string): Promise<Label[]> {
  const repo = await getRepo(ctx, owner, repoName);
  return ctx.db.all<Label>("SELECT id, name, color, description FROM labels WHERE repo_id = ?1 ORDER BY name ASC", [repo.id]);
}

export async function createLabel(ctx: AppContext, owner: string, repoName: string, input: any): Promise<Label> {
  const repo = await getRepo(ctx, owner, repoName);
  const name = String(input.name ?? "").trim();
  if (!name) throw ApiError.validation("Label", "name", "missing_field");
  const color = String(input.color ?? "ededed");
  const description = String(input.description ?? "");
  await ctx.db.execute(
    "INSERT INTO labels (repo_id, name, description, color) VALUES (?1, ?2, ?3, ?4) ON CONFLICT(repo_id, name) DO UPDATE SET description = excluded.description, color = excluded.color",
    [repo.id, name, description, color]
  );
  const row = await ctx.db.first<Label>("SELECT id, name, color, description FROM labels WHERE repo_id = ?1 AND name = ?2", [repo.id, name]);
  if (!row) throw ApiError.internal("failed to create label");
  return row;
}

async function ensureLabelIds(ctx: AppContext, repoId: number, names: string[]): Promise<number[]> {
  const ids: number[] = [];
  for (const rawName of names) {
    const name = String(rawName).trim();
    if (!name) continue;
    await ctx.db.execute(
      "INSERT INTO labels (repo_id, name, description, color) VALUES (?1, ?2, '', 'ededed') ON CONFLICT(repo_id, name) DO NOTHING",
      [repoId, name]
    );
    const row = await ctx.db.first<{ id: number }>("SELECT id FROM labels WHERE repo_id = ?1 AND name = ?2", [repoId, name]);
    if (row) ids.push(row.id);
  }
  return ids;
}

export async function listComments(ctx: AppContext, owner: string, repoName: string, number: number, query: URLSearchParams) {
  const issue = await resolveIssue(ctx, owner, repoName, number);
  const { page, perPage, offset } = normalizeCompatPagination(query);
  const filters = ["c.issue_id = ?1", "c.deleted_at IS NULL"];
  const params: any[] = [issue.issue_id];
  let idx = 2;
  const since = query.get("since");
  if (since) {
    filters.push(`c.updated_at >= ?${idx++}`);
    params.push(since);
  }
  const whereSql = filters.join(" AND ");
  const total = (await ctx.db.first<{ total: number }>(`SELECT COUNT(*) AS total FROM comments c WHERE ${whereSql}`, params))?.total ?? 0;
  const rows = await ctx.db.all<{ id: number }>(
    `SELECT c.id FROM comments c WHERE ${whereSql} ORDER BY c.created_at ASC LIMIT ?${idx} OFFSET ?${idx + 1}`,
    [...params, perPage, offset]
  );
  const items = [];
  for (const row of rows) items.push(await getComment(ctx, owner, repoName, row.id));
  return { items, total, page, perPage };
}

export async function createComment(ctx: AppContext, owner: string, repoName: string, number: number, input: any): Promise<CommentResponse> {
  const user = requireUser(ctx);
  const body = String(input.body ?? "");
  if (!body.trim()) throw ApiError.validation("IssueComment", "body", "missing_field");
  const issue = await resolveIssue(ctx, owner, repoName, number);
  const row = await ctx.db.first<{ id: number }>(
    "INSERT INTO comments (repo_id, issue_id, body, user_id, created_at, updated_at, reactions) VALUES (?1, ?2, ?3, ?4, datetime('now'), datetime('now'), '{}') RETURNING id",
    [issue.repo_id, issue.issue_id, body, user.id]
  );
  if (!row) throw ApiError.internal("comment insert failed");
  await ctx.db.execute("UPDATE issues SET comment_count = comment_count + 1, updated_at = datetime('now') WHERE id = ?1", [issue.issue_id]);
  return getComment(ctx, owner, repoName, row.id);
}

export async function getComment(ctx: AppContext, owner: string, repoName: string, commentId: number): Promise<CommentResponse> {
  const repo = await getRepo(ctx, owner, repoName);
  const row = await fetchCommentRow(ctx, repo.id, commentId);
  if (!row) throw ApiError.notFound("IssueComment");
  return buildCommentResponse(ctx, row);
}

export async function updateComment(ctx: AppContext, owner: string, repoName: string, commentId: number, input: any): Promise<CommentResponse> {
  const actor = requireUser(ctx);
  const body = String(input.body ?? "");
  if (!body.trim()) throw ApiError.validation("IssueComment", "body", "missing_field");
  const repo = await getRepo(ctx, owner, repoName);
  const row = await fetchCommentRow(ctx, repo.id, commentId);
  if (!row) throw ApiError.notFound("IssueComment");
  if (actor.id !== row.user_id && row.admin_user_id !== actor.id) throw ApiError.forbidden("You are not allowed to update this comment");
  await ctx.db.execute("UPDATE comments SET body = ?1, updated_at = datetime('now') WHERE id = ?2 AND deleted_at IS NULL", [body, commentId]);
  return getComment(ctx, owner, repoName, commentId);
}

export async function deleteComment(ctx: AppContext, owner: string, repoName: string, commentId: number): Promise<void> {
  const actor = requireUser(ctx);
  const repo = await getRepo(ctx, owner, repoName);
  const row = await fetchCommentRow(ctx, repo.id, commentId);
  if (!row) throw ApiError.notFound("IssueComment");
  if (actor.id !== row.user_id && row.admin_user_id !== actor.id) throw ApiError.forbidden("You are not allowed to delete this comment");
  await ctx.db.execute("UPDATE comments SET deleted_at = datetime('now'), updated_at = datetime('now') WHERE id = ?1 AND deleted_at IS NULL", [commentId]);
  await ctx.db.execute(
    "UPDATE issues SET comment_count = CASE WHEN comment_count > 0 THEN comment_count - 1 ELSE 0 END, updated_at = datetime('now') WHERE id = (SELECT issue_id FROM comments WHERE id = ?1)",
    [commentId]
  );
}

async function resolveIssue(ctx: AppContext, owner: string, repoName: string, number: number) {
  const repo = await getRepo(ctx, owner, repoName);
  const row = await ctx.db.first<{ issue_id: number; repo_id: number }>(
    "SELECT i.id AS issue_id, i.repo_id AS repo_id FROM issues i WHERE i.repo_id = ?1 AND i.number = ?2 AND i.deleted_at IS NULL",
    [repo.id, number]
  );
  if (!row) throw ApiError.notFound("Issue");
  return row;
}

async function fetchCommentRow(ctx: AppContext, repoId: number, commentId: number): Promise<CommentRow | null> {
  return ctx.db.first<CommentRow>(
    "SELECT c.id, c.issue_id, c.body, c.user_id, c.created_at, c.updated_at, c.reactions, u.login, u.avatar_url, u.type AS user_type, u.site_admin, i.number AS issue_number, r.owner AS repo_owner, r.name AS repo_name, r.admin_user_id FROM comments c JOIN users u ON u.id = c.user_id JOIN issues i ON i.id = c.issue_id JOIN repos r ON r.id = c.repo_id WHERE c.id = ?1 AND c.repo_id = ?2 AND c.deleted_at IS NULL",
    [commentId, repoId]
  );
}

function buildCommentResponse(ctx: AppContext, row: CommentRow): CommentResponse {
  const user: GitHubUser = {
    id: row.user_id,
    login: row.login,
    email: "",
    avatar_url: row.avatar_url,
    type: row.user_type,
    site_admin: row.site_admin !== 0
  };
  const issueUrl = `${ctx.baseUrl}/repos/${row.repo_owner}/${row.repo_name}/issues/${row.issue_number}`;
  return {
    id: row.id,
    node_id: base64Std(`atrium:Comment:${row.id}`),
    body: row.body,
    body_html: renderMarkdown(row.body),
    user: toApiUser(user),
    created_at: toIso(row.created_at)!,
    updated_at: toIso(row.updated_at)!,
    html_url: `${issueUrl}#comment-${row.id}`,
    issue_url: issueUrl,
    author_association: row.admin_user_id === row.user_id ? "OWNER" : "NONE",
    reactions: reactionPayload(ctx.baseUrl, row.repo_owner, row.repo_name, row.id, row.reactions)
  };
}

export async function listReactions(ctx: AppContext, owner: string, repoName: string, commentId: number, query: URLSearchParams) {
  await ensureComment(ctx, owner, repoName, commentId);
  const { page, perPage, offset } = normalizeCompatPagination(query);
  const total = (await ctx.db.first<{ total: number }>("SELECT COUNT(*) AS total FROM reactions WHERE comment_id = ?1", [commentId]))?.total ?? 0;
  const rows = await ctx.db.all<any>(
    "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type FROM reactions r JOIN users u ON u.id = r.user_id WHERE r.comment_id = ?1 ORDER BY r.id ASC LIMIT ?2 OFFSET ?3",
    [commentId, perPage, offset]
  );
  return {
    items: rows.map((row) => ({
      id: row.id,
      content: row.content,
      user: toApiUser({
        id: row.user_id,
        login: row.login,
        email: "",
        avatar_url: row.avatar_url,
        type: row.user_type,
        site_admin: false
      }),
      created_at: toIso(row.created_at)
    })),
    total,
    page,
    perPage
  };
}

export async function createReaction(ctx: AppContext, owner: string, repoName: string, commentId: number, input: any) {
  const user = requireUser(ctx);
  const content = String(input.content ?? "");
  if (!ALLOWED_REACTIONS.has(content)) throw ApiError.validation("Reaction", "content", "invalid");
  const comment = await ensureComment(ctx, owner, repoName, commentId);
  const affected = await ctx.db.execute(
    "INSERT INTO reactions (comment_id, user_id, content, created_at) VALUES (?1, ?2, ?3, datetime('now')) ON CONFLICT(comment_id, user_id, content) DO NOTHING",
    [commentId, user.id, content]
  );
  const row = await ctx.db.first<any>(
    "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type FROM reactions r JOIN users u ON u.id = r.user_id WHERE r.comment_id = ?1 AND r.user_id = ?2 AND r.content = ?3",
    [commentId, user.id, content]
  );
  if (!row) throw ApiError.internal("reaction create failed");
  if (affected > 0) await rebuildCachedReactions(ctx, commentId);
  return {
    reaction: {
      id: row.id,
      content: row.content,
      user: toApiUser({ id: row.user_id, login: row.login, email: "", avatar_url: row.avatar_url, type: row.user_type, site_admin: false }),
      created_at: toIso(row.created_at)
    },
    created: affected > 0,
    comment
  };
}

export async function deleteReaction(ctx: AppContext, owner: string, repoName: string, commentId: number, reactionId: number): Promise<void> {
  const actor = requireUser(ctx);
  await ensureComment(ctx, owner, repoName, commentId);
  const row = await ctx.db.first<any>(
    "SELECT r.id, r.content, r.user_id, r.created_at, u.login, u.avatar_url, u.type AS user_type FROM reactions r JOIN users u ON u.id = r.user_id WHERE r.id = ?1 AND r.comment_id = ?2",
    [reactionId, commentId]
  );
  if (!row) throw ApiError.notFound("Reaction");
  if (row.user_id !== actor.id) throw ApiError.forbidden("You can only delete your own reaction");
  const affected = await ctx.db.execute("DELETE FROM reactions WHERE id = ?1 AND comment_id = ?2", [reactionId, commentId]);
  if (affected > 0) await rebuildCachedReactions(ctx, commentId);
}

export async function deleteReactionByContent(ctx: AppContext, owner: string, repoName: string, commentId: number, content: string): Promise<void> {
  const user = requireUser(ctx);
  const repo = await getRepo(ctx, owner, repoName);
  const row = await ctx.db.first<{ id: number }>(
    "SELECT r.id FROM reactions r JOIN comments c ON c.id = r.comment_id WHERE c.repo_id = ?1 AND r.comment_id = ?2 AND r.user_id = ?3 AND r.content = ?4",
    [repo.id, commentId, user.id, content]
  );
  if (row) await deleteReaction(ctx, owner, repoName, commentId, row.id);
}

async function ensureComment(ctx: AppContext, owner: string, repoName: string, commentId: number) {
  const repo = await getRepo(ctx, owner, repoName);
  const row = await ctx.db.first<{ issue_id: number }>(
    "SELECT c.issue_id FROM comments c WHERE c.id = ?1 AND c.repo_id = ?2 AND c.deleted_at IS NULL",
    [commentId, repo.id]
  );
  if (!row) throw ApiError.notFound("IssueComment");
  return row;
}

async function rebuildCachedReactions(ctx: AppContext, commentId: number) {
  const row = await ctx.db.first<Omit<ReactionCounts, "total">>(
    "SELECT COALESCE(SUM(CASE WHEN content = '+1' THEN 1 ELSE 0 END), 0) AS plus_one, COALESCE(SUM(CASE WHEN content = '-1' THEN 1 ELSE 0 END), 0) AS minus_one, COALESCE(SUM(CASE WHEN content = 'laugh' THEN 1 ELSE 0 END), 0) AS laugh, COALESCE(SUM(CASE WHEN content = 'confused' THEN 1 ELSE 0 END), 0) AS confused, COALESCE(SUM(CASE WHEN content = 'heart' THEN 1 ELSE 0 END), 0) AS heart, COALESCE(SUM(CASE WHEN content = 'hooray' THEN 1 ELSE 0 END), 0) AS hooray, COALESCE(SUM(CASE WHEN content = 'rocket' THEN 1 ELSE 0 END), 0) AS rocket, COALESCE(SUM(CASE WHEN content = 'eyes' THEN 1 ELSE 0 END), 0) AS eyes FROM reactions WHERE comment_id = ?1",
    [commentId]
  );
  const counts: ReactionCounts = {
    plus_one: Number(row?.plus_one ?? 0),
    minus_one: Number(row?.minus_one ?? 0),
    laugh: Number(row?.laugh ?? 0),
    confused: Number(row?.confused ?? 0),
    heart: Number(row?.heart ?? 0),
    hooray: Number(row?.hooray ?? 0),
    rocket: Number(row?.rocket ?? 0),
    eyes: Number(row?.eyes ?? 0),
    total: 0
  };
  counts.total = counts.plus_one + counts.minus_one + counts.laugh + counts.confused + counts.heart + counts.hooray + counts.rocket + counts.eyes;
  await ctx.db.execute("UPDATE comments SET reactions = ?1, updated_at = updated_at WHERE id = ?2", [JSON.stringify(counts), commentId]);
}

export async function searchIssues(ctx: AppContext, query: URLSearchParams) {
  const parsed = parseSearchQuery(query.get("q") ?? "");
  const { page, perPage, offset } = normalizeCompatPagination(query);
  const filters = ["i.deleted_at IS NULL"];
  const params: any[] = [];
  let idx = 1;
  if (parsed.owner && parsed.repo) {
    filters.push(`(lower(r.owner) = lower(?${idx}) OR r.owner_user_id = (SELECT u.id FROM users u JOIN user_identities ui ON ui.user_id = u.id WHERE ui.provider = 'github' AND lower(u.login) = lower(?${idx}) LIMIT 1))`);
    params.push(parsed.owner);
    idx += 1;
    filters.push(`r.name = ?${idx++}`);
    params.push(parsed.repo);
  }
  if (parsed.state) {
    filters.push(`i.state = ?${idx++}`);
    params.push(parsed.state);
  }
  if (parsed.label) {
    filters.push(`EXISTS (SELECT 1 FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE il.issue_id = i.id AND l.name = ?${idx++})`);
    params.push(parsed.label);
  }
  if (parsed.text) {
    filters.push(`(INSTR(LOWER(i.title), LOWER(?${idx})) > 0 OR INSTR(LOWER(COALESCE(i.body, '')), LOWER(?${idx + 1})) > 0)`);
    params.push(parsed.text, parsed.text);
    idx += 2;
  }
  const whereSql = filters.join(" AND ");
  const total = (await ctx.db.first<{ total: number }>(`SELECT COUNT(*) AS total FROM issues i JOIN repos r ON r.id = i.repo_id WHERE ${whereSql}`, params))?.total ?? 0;
  const sortCol = query.get("sort") === "updated" ? "i.updated_at" : query.get("sort") === "comments" ? "i.comment_count" : "i.created_at";
  const order = query.get("order") === "asc" ? "ASC" : "DESC";
  const rows = await ctx.db.all<{ owner: string; repo: string; number: number }>(
    `SELECT r.owner AS owner, r.name AS repo, i.number AS number FROM issues i JOIN repos r ON r.id = i.repo_id WHERE ${whereSql} ORDER BY ${sortCol} ${order} LIMIT ?${idx} OFFSET ?${idx + 1}`,
    [...params, perPage, offset]
  );
  const items = [];
  for (const row of rows) items.push(await getIssue(ctx, row.owner, row.repo, row.number));
  return { items, total, page, perPage };
}

function parseSearchQuery(q: string) {
  const parsed: { owner?: string; repo?: string; label?: string; state?: string; text: string } = { text: "" };
  const text: string[] = [];
  for (const raw of q.split(/\s+/)) {
    const token = raw.trim();
    if (!token) continue;
    if (token.startsWith("repo:")) {
      const [owner, repo] = token.slice(5).split("/");
      if (owner && repo) {
        parsed.owner = owner;
        parsed.repo = repo;
        continue;
      }
    }
    if (token.startsWith("label:")) {
      parsed.label = token.slice(6);
      continue;
    }
    if (token === "is:open" || token === "is:closed") {
      parsed.state = token.slice(3);
      continue;
    }
    if (token.startsWith("type:") || token.startsWith("in:")) continue;
    if (/^[A-Za-z0-9_]+:[^/].+/.test(token)) continue;
    const cleaned = token.replace(/^['"]|['"]$/g, "");
    if (cleaned) text.push(cleaned);
  }
  parsed.text = text.join(" ");
  return parsed;
}

export async function nativeListThreads(ctx: AppContext, owner: string, repoName: string, query: URLSearchParams) {
  await getRepo(ctx, owner, repoName);
  const limit = Math.min(100, Math.max(1, Number.parseInt(query.get("limit") ?? "20", 10) || 20));
  const direction = query.get("direction") ?? "desc";
  const cursor = query.get("cursor");
  const cursorId = cursor ? decodeCursor(cursor) : null;
  const filters = ["r.owner = ?1", "r.name = ?2", "i.deleted_at IS NULL"];
  const params: any[] = [owner, repoName];
  let idx = 3;
  const state = query.get("state") ?? "open";
  if (state !== "all") {
    filters.push(`i.state = ?${idx++}`);
    params.push(state);
  }
  const slug = query.get("slug")?.trim();
  if (slug) {
    filters.push(`i.slug = ?${idx++}`);
    params.push(slug);
  }
  const title = query.get("title")?.trim();
  if (title) {
    filters.push(`i.title = ?${idx++}`);
    params.push(title);
  }
  if (cursorId != null) {
    filters.push(direction.toLowerCase() === "asc" ? `i.id > ?${idx++}` : `i.id < ?${idx++}`);
    params.push(cursorId);
  }
  const order = direction.toLowerCase() === "asc" ? "ASC" : "DESC";
  const pointers = await ctx.db.all<{ id: number; number: number }>(
    `SELECT i.id, i.number FROM issues i JOIN repos r ON r.id = i.repo_id WHERE ${filters.join(" AND ")} ORDER BY i.id ${order} LIMIT ?${idx}`,
    [...params, limit + 1]
  );
  const hasMore = pointers.length > limit;
  if (hasMore) pointers.pop();
  const data = [];
  for (const pointer of pointers) data.push(toNativeThread(await getIssue(ctx, owner, repoName, pointer.number)));
  return {
    data,
    pagination: {
      next_cursor: hasMore && pointers.length ? encodeCursor(pointers[pointers.length - 1]!.id) : null,
      has_more: hasMore
    }
  };
}

export async function nativeListComments(ctx: AppContext, owner: string, repoName: string, number: number, query: URLSearchParams) {
  const issue = await getIssue(ctx, owner, repoName, number);
  const limit = Math.min(100, Math.max(1, Number.parseInt(query.get("limit") ?? "20", 10) || 20));
  const order = query.get("order") ?? "asc";
  const cursor = query.get("cursor");
  const cursorId = cursor ? decodeCursor(cursor) : null;
  const filters = ["c.issue_id = ?1", "c.deleted_at IS NULL"];
  const params: any[] = [issue.id];
  let idx = 2;
  if (cursorId != null) {
    filters.push(order.toLowerCase() === "desc" ? `c.id < ?${idx++}` : `c.id > ?${idx++}`);
    params.push(cursorId);
  }
  const orderSql = order.toLowerCase() === "desc" ? "DESC" : "ASC";
  const pointers = await ctx.db.all<{ id: number }>(
    `SELECT c.id FROM comments c WHERE ${filters.join(" AND ")} ORDER BY c.id ${orderSql} LIMIT ?${idx}`,
    [...params, limit + 1]
  );
  const hasMore = pointers.length > limit;
  if (hasMore) pointers.pop();
  const data = [];
  for (const pointer of pointers) data.push(toNativeComment(await getComment(ctx, owner, repoName, pointer.id)));
  return {
    data,
    pagination: {
      next_cursor: hasMore && pointers.length ? encodeCursor(pointers[pointers.length - 1]!.id) : null,
      has_more: hasMore
    }
  };
}

export function toNativeThread(issue: IssueResponse) {
  return {
    id: issue.id,
    number: issue.number,
    title: issue.title,
    ...(issue.slug ? { slug: issue.slug } : {}),
    body: issue.body ?? "",
    body_html: issue.body_html ?? "",
    state: issue.state,
    comment_count: issue.comments,
    author: { id: issue.user.id, login: issue.user.login, avatar_url: issue.user.avatar_url, email: "" },
    labels: issue.labels.map((label) => ({ id: label.id, name: label.name, color: label.color })),
    reactions: toNativeReactions(issue.reactions),
    created_at: issue.created_at,
    updated_at: issue.updated_at
  };
}

export function toNativeComment(comment: CommentResponse) {
  return {
    id: comment.id,
    body: comment.body ?? "",
    body_html: comment.body_html ?? "",
    author: { id: comment.user.id, login: comment.user.login, avatar_url: comment.user.avatar_url, email: "" },
    reactions: toNativeReactions(comment.reactions),
    created_at: comment.created_at,
    updated_at: comment.updated_at
  };
}

function toNativeReactions(reactions: any) {
  return {
    "+1": reactions["+1"] ?? 0,
    "-1": reactions["-1"] ?? 0,
    laugh: reactions.laugh ?? 0,
    confused: reactions.confused ?? 0,
    heart: reactions.heart ?? 0,
    hooray: reactions.hooray ?? 0,
    rocket: reactions.rocket ?? 0,
    eyes: reactions.eyes ?? 0,
    total: reactions.total_count ?? 0
  };
}

export async function exportNativeRepo(ctx: AppContext, owner: string, repoName: string, query: URLSearchParams) {
  const actor = requireUser(ctx);
  const repo = await getRepo(ctx, owner, repoName);
  if (repo.admin_user_id !== actor.id) throw ApiError.forbidden("Admin required");
  const since = normalizeSince(query.get("since"));
  const format = query.get("format") ?? "json";
  if (format.toLowerCase() !== "json" && format.toLowerCase() !== "csv") throw ApiError.badRequest("invalid format, expected json or csv");
  const labels = await ctx.db.all<any>("SELECT id, name, color FROM labels WHERE repo_id = ?1 ORDER BY id ASC", [repo.id]);
  let threadSql =
    "SELECT i.id, i.number, i.title, i.body, i.state, u.id AS author_id, u.login AS author_login, i.created_at, i.updated_at FROM issues i JOIN users u ON u.id = i.user_id WHERE i.repo_id = ?1 AND i.deleted_at IS NULL";
  const threadParams: any[] = [repo.id];
  if (since) {
    threadSql += " AND i.updated_at >= ?2";
    threadParams.push(since);
  }
  threadSql += " ORDER BY i.number ASC";
  const threads = await ctx.db.all<any>(threadSql, threadParams);
  const labelRows = await ctx.db.all<any>(
    "SELECT il.issue_id, l.name FROM issue_labels il JOIN labels l ON l.id = il.label_id WHERE l.repo_id = ?1 ORDER BY il.issue_id ASC, l.name ASC",
    [repo.id]
  );
  const labelsByIssue = new Map<number, string[]>();
  for (const row of labelRows) {
    if (!labelsByIssue.has(row.issue_id)) labelsByIssue.set(row.issue_id, []);
    labelsByIssue.get(row.issue_id)!.push(row.name);
  }
  let commentSql =
    "SELECT c.id, c.issue_id, c.body, u.id AS author_id, u.login AS author_login, c.created_at, c.updated_at, c.reactions FROM comments c JOIN users u ON u.id = c.user_id WHERE c.repo_id = ?1 AND c.deleted_at IS NULL";
  const commentParams: any[] = [repo.id];
  if (since) {
    commentSql += " AND c.updated_at >= ?2";
    commentParams.push(since);
  }
  commentSql += " ORDER BY c.issue_id ASC, c.id ASC";
  const comments = await ctx.db.all<any>(commentSql, commentParams);
  const commentsByIssue = new Map<number, any[]>();
  for (const row of comments) {
    const counts = parseReactionCounts(row.reactions);
    if (!commentsByIssue.has(row.issue_id)) commentsByIssue.set(row.issue_id, []);
    commentsByIssue.get(row.issue_id)!.push({
      id: row.id,
      body: row.body,
      author: { id: row.author_id, login: row.author_login },
      reactions: {
        "+1": counts.plus_one,
        "-1": counts.minus_one,
        laugh: counts.laugh,
        confused: counts.confused,
        heart: counts.heart,
        hooray: counts.hooray,
        rocket: counts.rocket,
        eyes: counts.eyes,
        total: counts.total
      },
      created_at: toIso(row.created_at),
      updated_at: toIso(row.updated_at)
    });
  }
  const exportThreads = threads.map((thread) => ({
    number: thread.number,
    title: thread.title,
    body: thread.body ?? "",
    state: thread.state,
    author: { id: thread.author_id, login: thread.author_login },
    labels: labelsByIssue.get(thread.id) ?? [],
    created_at: toIso(thread.created_at),
    updated_at: toIso(thread.updated_at),
    comments: commentsByIssue.get(thread.id) ?? []
  }));
  const payload = {
    repo: { owner, name: repoName },
    exported_at: isoNow(),
    labels: labels.map((label) => ({ id: label.id, name: label.name, color: label.color })),
    threads: exportThreads
  };
  if (format.toLowerCase() === "csv") return { csv: buildExportCsv(owner, repoName, exportThreads) };
  return { json: payload };
}

export async function exportUserRepos(ctx: AppContext) {
  const user = requireUser(ctx);
  const params = [user.login, user.id];
  const scopeSql = "SELECT id FROM repos WHERE lower(owner) = lower(?1) OR owner_user_id = ?2 OR admin_user_id = ?2";
  const repos = await ctx.db.all<any>(
    "SELECT id, owner, name, owner_user_id, admin_user_id, issue_counter, created_at FROM repos WHERE lower(owner) = lower(?1) OR owner_user_id = ?2 OR admin_user_id = ?2 ORDER BY id ASC",
    params
  );
  const issues = await ctx.db.all<any>(
    `SELECT id, repo_id, number, title, body, state, state_reason, locked, user_id, comment_count, created_at, updated_at, closed_at, deleted_at FROM issues WHERE repo_id IN (${scopeSql}) ORDER BY id ASC`,
    params
  );
  const comments = await ctx.db.all<any>(
    `SELECT id, repo_id, issue_id, body, user_id, created_at, updated_at, deleted_at, reactions FROM comments WHERE repo_id IN (${scopeSql}) ORDER BY id ASC`,
    params
  );
  const labels = await ctx.db.all<any>(
    `SELECT id, repo_id, name, description, color FROM labels WHERE repo_id IN (${scopeSql}) ORDER BY id ASC`,
    params
  );
  const issueLabels = await ctx.db.all<any>(
    `SELECT il.issue_id, il.label_id FROM issue_labels il JOIN issues i ON i.id = il.issue_id WHERE i.repo_id IN (${scopeSql}) ORDER BY il.issue_id ASC, il.label_id ASC`,
    params
  );
  const reactions = await ctx.db.all<any>(
    `SELECT r.id, r.comment_id, r.user_id, r.content, r.created_at FROM reactions r JOIN comments c ON c.id = r.comment_id WHERE c.repo_id IN (${scopeSql}) ORDER BY r.id ASC`,
    params
  );
  const users = await ctx.db.all<any>(
    `SELECT DISTINCT u.id, u.login, u.email, u.avatar_url, u.type, u.site_admin, u.cached_at FROM users u WHERE u.id IN (SELECT admin_user_id FROM repos WHERE (lower(owner) = lower(?1) OR owner_user_id = ?2 OR admin_user_id = ?2) AND admin_user_id IS NOT NULL UNION SELECT owner_user_id FROM repos WHERE (lower(owner) = lower(?1) OR owner_user_id = ?2 OR admin_user_id = ?2) AND owner_user_id IS NOT NULL UNION SELECT i.user_id FROM issues i WHERE i.repo_id IN (${scopeSql}) UNION SELECT c.user_id FROM comments c WHERE c.repo_id IN (${scopeSql}) UNION SELECT r.user_id FROM reactions r JOIN comments c ON c.id = r.comment_id WHERE c.repo_id IN (${scopeSql})) ORDER BY u.id ASC`,
    params
  );
  return {
    schema_version: 1,
    exported_at: isoNow(),
    user,
    repos,
    issues,
    comments,
    labels,
    issue_labels: issueLabels,
    reactions,
    users
  };
}

function normalizeSince(value: string | null): string | null {
  if (!value) return null;
  const time = Date.parse(value);
  if (Number.isNaN(time)) throw ApiError.badRequest("invalid since parameter");
  return new Date(time).toISOString().replace(/\.\d{3}Z$/, "Z");
}

function csvEscape(value: unknown): string {
  const text = String(value ?? "");
  if (/[",\n\r]/.test(text)) return `"${text.replace(/"/g, '""')}"`;
  return text;
}

function buildExportCsv(owner: string, repo: string, threads: any[]): string {
  let out =
    "repo_owner,repo_name,thread_number,thread_title,thread_state,thread_author_id,thread_author_login,thread_created_at,thread_updated_at,comment_id,comment_author_id,comment_author_login,comment_body,comment_created_at,comment_updated_at,comment_reactions,labels\n";
  for (const thread of threads) {
    const labels = thread.labels.join("|");
    if (thread.comments.length === 0) {
      out += [
        owner,
        repo,
        thread.number,
        thread.title,
        thread.state,
        thread.author.id,
        thread.author.login,
        thread.created_at,
        thread.updated_at,
        "",
        "",
        "",
        "",
        "",
        "",
        "{}",
        labels
      ].map(csvEscape).join(",") + "\n";
    } else {
      for (const comment of thread.comments) {
        out += [
          owner,
          repo,
          thread.number,
          thread.title,
          thread.state,
          thread.author.id,
          thread.author.login,
          thread.created_at,
          thread.updated_at,
          comment.id,
          comment.author.id,
          comment.author.login,
          comment.body,
          comment.created_at,
          comment.updated_at,
          JSON.stringify(comment.reactions),
          labels
        ].map(csvEscape).join(",") + "\n";
      }
    }
  }
  return out;
}
