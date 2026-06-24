import { describe, expect, test } from "vitest";

const baseUrl = process.env.ATRIUM_TEST_BASE_URL ?? "http://127.0.0.1:8788";
const bypassSecret = process.env.ATRIUM_TEST_BYPASS_SECRET ?? "atrium-test";

class Client {
  constructor(private readonly auth?: string) {}
  get(path: string, headers?: Record<string, string>) {
    return this.request("GET", path, undefined, headers);
  }
  post(path: string, body?: unknown, headers?: Record<string, string>) {
    return this.request("POST", path, body, headers);
  }
  put(path: string, body?: unknown, headers?: Record<string, string>) {
    return this.request("PUT", path, body, headers);
  }
  patch(path: string, body?: unknown, headers?: Record<string, string>) {
    return this.request("PATCH", path, body, headers);
  }
  delete(path: string, headers?: Record<string, string>) {
    return this.request("DELETE", path, undefined, headers);
  }
  private request(method: string, path: string, body?: unknown, headers?: Record<string, string>) {
    const requestHeaders: Record<string, string> = { ...(headers ?? {}) };
    if (this.auth) requestHeaders.Authorization = this.auth;
    if (body !== undefined) requestHeaders["Content-Type"] = "application/json";
    return fetch(`${baseUrl}${path}`, {
      method,
      headers: requestHeaders,
      body: body === undefined ? undefined : JSON.stringify(body)
    });
  }
}

const anon = new Client();
const superAdmin = user(1, "super", "acct-super");
const owner = user(2, "owner", "acct-owner");
const alice = user(3, "alice", "acct-alice");
const bob = user(4, "bob", "acct-bob");

function user(id: number, login: string, accountSub: string) {
  return new Client(`testuser ${bypassSecret}:${id}:${login}:${login}@test.com:${accountSub}`);
}

async function json(response: Response) {
  return (await response.json()) as any;
}

async function ensureUsers() {
  for (const client of [superAdmin, owner, alice, bob]) {
    const response = await client.get("/api/v1/auth/me");
    expect(response.status).toBe(200);
  }
}

async function createWebsite(key: string, origin: string) {
  const created = await superAdmin.post("/api/v1/websites", {
    key,
    name: key,
    origins: [origin],
    admin_user_ids: [2]
  });
  expect(created.status).toBe(201);
}

describe("Atrium native Worker API", () => {
  test("root and removed GitHub-compatible routes", async () => {
    const root = await anon.get("/");
    expect(root.status).toBe(200);
    const rootText = await root.text();
    expect(rootText).toContain("website/page/comment");
    expect(rootText).toContain("站点接入");
    expect(rootText).toContain("/docs/discovery");
    expect(rootText).toContain("_atrium.<host> TXT");
    expect(rootText).toContain("不需要声明 website_key");
    expect(rootText).toContain("origin 可省略");

    const discoveryKey = await anon.get("/api/v1/discovery/public-key");
    expect(discoveryKey.status).toBe(200);
    expect((await json(discoveryKey)).alg).toBe("RSA-OAEP-256");

    const guide = await anon.get("/docs/discovery");
    expect(guide.status).toBe(200);
    expect(guide.headers.get("Content-Type")).toContain("text/html");
    const guideHtml = await guide.text();
    expect(guideHtml).toContain("Atrium Discovery 接入指南");
    expect(guideHtml).toContain("/.well-known/atrium.json");
    expect(guideHtml).toContain("_atrium.blog.example.com TXT");
    expect(guideHtml).toContain("/api/v1/discovery/public-key");
    expect(guideHtml).toContain("可选填写 origin");
    expect(guideHtml).not.toContain('"website_key"');

    const corsOrigin = "https://comments.example.com";
    const corsResponse = await anon.get("/api/v1/auth/me", { Origin: corsOrigin });
    expect(corsResponse.headers.get("Access-Control-Allow-Origin")).toBe(corsOrigin);
    expect(corsResponse.headers.get("Access-Control-Allow-Credentials")).toBe("true");
    expect(corsResponse.headers.get("Vary")).toContain("Origin");

    const preflight = await fetch(`${baseUrl}/api/v1/auth/me`, {
      method: "OPTIONS",
      headers: {
        Origin: corsOrigin,
        "Access-Control-Request-Method": "GET"
      }
    });
    expect(preflight.status).toBe(204);
    expect(preflight.headers.get("Access-Control-Allow-Origin")).toBe(corsOrigin);
    expect(preflight.headers.get("Access-Control-Allow-Credentials")).toBe("true");

    expect((await anon.get("/repos/e2e/repo/issues")).status).toBe(404);
    expect((await anon.post("/repos/e2e/repo/issues", { title: "old" })).status).toBe(404);
    expect((await anon.get("/api/v1/repos/e2e/repo/threads")).status).toBe(404);
    expect((await anon.get("/search/issues?q=x")).status).toBe(404);
    expect((await anon.post("/markdown", { text: "**old**" })).status).toBe(404);
  });

  test("super admin env gates website creation and website admin management", async () => {
    await ensureUsers();

    expect((await json(await superAdmin.get("/api/v1/auth/me"))).super_admin).toBe(true);
    expect((await json(await owner.get("/api/v1/auth/me"))).super_admin).toBe(false);
    expect((await owner.post("/api/v1/websites", { key: "nope", name: "Nope" })).status).toBe(403);

    const created = await superAdmin.post("/api/v1/websites", {
      key: "admin.blog.example.com",
      name: "Blog",
      origins: ["https://blog.example.com"],
      admin_user_ids: [2]
    });
    expect(created.status).toBe(201);
    expect((await json(created)).origins).toEqual(["https://blog.example.com"]);

    const patched = await owner.patch("/api/v1/websites/admin.blog.example.com", {
      name: "Blog Updated",
      origins: ["https://blog.example.com", "https://www.blog.example.com"]
    });
    expect(patched.status).toBe(200);
    expect((await json(patched)).name).toBe("Blog Updated");

    const admins = await owner.get("/api/v1/websites/admin.blog.example.com/admins");
    expect(admins.status).toBe(200);
    expect((await json(admins)).data.map((entry: any) => entry.user.id)).toContain(2);

    expect((await owner.delete("/api/v1/websites/admin.blog.example.com/admins/1")).status).toBe(204);
    expect((await owner.delete("/api/v1/websites/admin.blog.example.com/admins/2")).status).toBe(403);
    expect((await superAdmin.post("/api/v1/websites/admin.blog.example.com/admins", { user_id: 1 })).status).toBe(201);
  });

  test("explicit page, comments, replies, reactions, moderation, and bans", async () => {
    await ensureUsers();
    await createWebsite("explicit-blog", "https://explicit.example.com");

    const page = await owner.put("/api/v1/websites/explicit-blog/pages/post-1", {
      title: "Post 1",
      url: "https://explicit.example.com/post-1?b=2&a=1#section",
      metadata: { source: "test" }
    });
    expect(page.status).toBe(200);
    const pageBody = await json(page);
    expect(pageBody.normalized_url).toBe("https://explicit.example.com/post-1?a=1&b=2");

    const comment = await alice.post("/api/v1/websites/explicit-blog/pages/post-1/comments", { body: "hello" });
    expect(comment.status).toBe(201);
    const commentId = (await json(comment)).id;

    const reply = await bob.post("/api/v1/websites/explicit-blog/pages/post-1/comments", { body: "reply", parent_id: commentId });
    expect(reply.status).toBe(201);
    const replyId = (await json(reply)).id;

    const roots = await anon.get("/api/v1/websites/explicit-blog/pages/post-1/comments?parent_id=root");
    expect(roots.status).toBe(200);
    expect((await json(roots)).data.map((item: any) => item.id)).toContain(commentId);

    const replies = await anon.get(`/api/v1/websites/explicit-blog/pages/post-1/comments?parent_id=${commentId}`);
    expect(replies.status).toBe(200);
    expect((await json(replies)).data.map((item: any) => item.id)).toEqual([replyId]);

    expect((await bob.patch(`/api/v1/websites/explicit-blog/comments/${commentId}`, { body: "bob edit" })).status).toBe(403);
    const edited = await alice.patch(`/api/v1/websites/explicit-blog/comments/${commentId}`, { body: "alice edit" });
    expect(edited.status).toBe(200);
    expect((await json(edited)).body).toBe("alice edit");

    const reaction = await bob.put(`/api/v1/websites/explicit-blog/comments/${commentId}/reactions/heart`);
    expect(reaction.status).toBe(200);
    expect((await json(reaction)).heart).toBe(1);
    const duplicateReaction = await bob.put(`/api/v1/websites/explicit-blog/comments/${commentId}/reactions/heart`);
    expect((await json(duplicateReaction)).heart).toBe(1);
    expect((await bob.put(`/api/v1/websites/explicit-blog/comments/${commentId}/reactions/invalid`)).status).toBe(422);
    expect((await bob.delete(`/api/v1/websites/explicit-blog/comments/${commentId}/reactions/heart`)).status).toBe(204);

    expect((await owner.delete(`/api/v1/websites/explicit-blog/comments/${replyId}`)).status).toBe(204);
    const moderation = await owner.get("/api/v1/websites/explicit-blog/admin/comments?status=deleted&page_key=post-1");
    expect(moderation.status).toBe(200);
    expect((await json(moderation)).data.map((item: any) => item.id)).toContain(replyId);

    const ban = await owner.post("/api/v1/websites/explicit-blog/bans", { user_id: 3, reason: "spam" });
    expect(ban.status).toBe(201);
    expect((await json(ban)).data.map((entry: any) => entry.user.id)).toContain(3);
    expect((await alice.post("/api/v1/websites/explicit-blog/pages/post-1/comments", { body: "blocked" })).status).toBe(403);
    expect((await alice.patch(`/api/v1/websites/explicit-blog/comments/${commentId}`, { body: "blocked" })).status).toBe(403);
    expect((await alice.put(`/api/v1/websites/explicit-blog/comments/${commentId}/reactions/like`)).status).toBe(403);
    expect((await anon.get("/api/v1/websites/explicit-blog/pages/post-1/comments?parent_id=root")).status).toBe(200);
    expect((await owner.delete("/api/v1/websites/explicit-blog/bans/3")).status).toBe(204);
    expect((await alice.post("/api/v1/websites/explicit-blog/pages/post-1/comments", { body: "after unban" })).status).toBe(201);
  });

  test("quick Referer mode resolves website and page without creating websites", async () => {
    await ensureUsers();
    await createWebsite("quick-blog", "https://quick.example.com");

    expect((await anon.get("/api/v1/comments/current")).status).toBe(400);
    const unmatched = await anon.get("/api/v1/comments/current", { Referer: "https://unknown.example.com/post" });
    expect(unmatched.status).toBe(404);
    expect((await json(unmatched)).message).toBe("website_not_found");

    const referer = "https://quick.example.com/quick-post?z=9&a=1#comments";
    const current = await anon.get("/api/v1/comments/current?page_title=Quick%20Post", { Referer: referer });
    expect(current.status).toBe(200);
    const currentBody = await json(current);
    expect(currentBody.website.key).toBe("quick-blog");
    expect(currentBody.page.key).toMatch(/^url-/);
    expect(currentBody.page.normalized_url).toBe("https://quick.example.com/quick-post?a=1&z=9");

    const quickComment = await alice.post("/api/v1/comments/current", { body: "quick" }, { Referer: referer });
    expect(quickComment.status).toBe(201);
    const quickCommentId = (await json(quickComment)).id;

    const quickReply = await bob.post("/api/v1/comments/current", { body: "quick reply", parent_id: quickCommentId }, { Referer: referer });
    expect(quickReply.status).toBe(201);
    const quickReplyId = (await json(quickReply)).id;

    const replies = await anon.get(`/api/v1/comments/current/replies?comment_id=${quickCommentId}`, { Referer: referer });
    expect(replies.status).toBe(200);
    expect((await json(replies)).data.map((item: any) => item.id)).toEqual([quickReplyId]);

    const reaction = await bob.put(`/api/v1/comments/current/${quickCommentId}/reactions/like`, undefined, { Referer: referer });
    expect(reaction.status).toBe(200);
    expect((await json(reaction)).like).toBe(1);

    expect((await owner.post("/api/v1/websites/quick-blog/bans", { user_id: 3, reason: "quick spam" })).status).toBe(201);
    expect((await alice.post("/api/v1/comments/current", { body: "blocked" }, { Referer: referer })).status).toBe(403);
    expect((await alice.put(`/api/v1/comments/current/${quickCommentId}/reactions/heart`, undefined, { Referer: referer })).status).toBe(403);
    expect((await anon.get("/api/v1/comments/current", { Referer: referer })).status).toBe(200);
  });

  test("quick Referer mode discovers unknown origins from well-known metadata", async () => {
    await ensureUsers();

    await expectDiscoveredWebsite("https://discover-file.example.com", "discover-file.example.com");
    await expectDiscoveredWebsite("https://discover-file-encrypted.example.com", "discover-file-encrypted.example.com");
  });

  test("quick Referer mode discovers unknown origins from DNS TXT metadata", async () => {
    await ensureUsers();

    await expectDiscoveredWebsite("https://discover-dns.example.com", "discover-dns.example.com");
    await expectDiscoveredWebsite("https://discover-dns-encrypted.example.com", "discover-dns-encrypted.example.com");
  });

  test("discovery rejects unsafe or conflicting metadata", async () => {
    await ensureUsers();
    await createWebsite("discover-conflict.example.com", "https://registered-key.example.com");

    const cases = [
      "https://discover-mismatch.example.com/post",
      "https://discover-bad-jwe.example.com/post",
      "https://discover-wrong-type.example.com/post",
      "https://discover-conflict.example.com/post"
    ];

    for (const referer of cases) {
      const response = await anon.get("/api/v1/comments/current", { Referer: referer });
      expect(response.status).toBe(404);
      expect((await json(response)).message).toBe("website_not_found");
    }
  });
});

async function expectDiscoveredWebsite(origin: string, websiteKey: string) {
  const referer = `${origin}/post?z=9&a=1#comments`;
  const response = await anon.get("/api/v1/comments/current?page_title=Discovered", { Referer: referer });
  expect(response.status).toBe(200);
  const body = await json(response);
  expect(body.website.key).toBe(websiteKey);
  expect(body.website.origins).toEqual([origin]);
  expect(body.page.normalized_url).toBe(`${origin}/post?a=1&z=9`);

  const admins = await owner.get(`/api/v1/websites/${websiteKey}/admins`);
  expect(admins.status).toBe(200);
  expect((await json(admins)).data.map((entry: any) => entry.user.email)).toContain("owner@test.com");
}
