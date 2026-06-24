import { describe, expect, test } from "vitest";

const baseUrl = process.env.ATRIUM_TEST_BASE_URL ?? "http://127.0.0.1:8788";
const bypassSecret = process.env.ATRIUM_TEST_BYPASS_SECRET ?? "atrium-test";

class Client {
  constructor(private readonly auth?: string) {}
  get(path: string) {
    return this.request("GET", path);
  }
  post(path: string, body?: unknown) {
    return this.request("POST", path, body);
  }
  patch(path: string, body?: unknown) {
    return this.request("PATCH", path, body);
  }
  delete(path: string) {
    return this.request("DELETE", path);
  }
  private request(method: string, path: string, body?: unknown) {
    const headers: Record<string, string> = {};
    if (this.auth) headers.Authorization = this.auth;
    if (body !== undefined) headers["Content-Type"] = "application/json";
    return fetch(`${baseUrl}${path}`, {
      method,
      headers,
      body: body === undefined ? undefined : JSON.stringify(body)
    });
  }
}

const anon = new Client();
const admin = user(1, "admin");
const alice = user(2, "alice");
const bob = user(3, "bob");

function user(id: number, login: string) {
  return new Client(`testuser ${bypassSecret}:${id}:${login}:${login}@test.com`);
}

async function json(response: Response) {
  return await response.json() as any;
}

async function seedIssue(client: Client, owner: string, repo: string, title: string) {
  const response = await client.post(`/repos/${owner}/${repo}/issues`, { title, body: "body" });
  expect(response.status).toBe(201);
  return (await json(response)).number as number;
}

async function seedComment(client: Client, owner: string, repo: string, number: number, body: string) {
  const response = await client.post(`/repos/${owner}/${repo}/issues/${number}/comments`, { body });
  expect(response.status).toBe(201);
  return (await json(response)).id as number;
}

describe("Atrium Worker API", () => {
  test("root, markdown, user, and user export", async () => {
    const root = await anon.get("/");
    expect(root.status).toBe(200);
    expect(await root.text()).toContain("Atrium");

    const markdown = await anon.post("/markdown", { text: "**hello**" });
    expect(markdown.status).toBe(200);
    expect(await markdown.text()).toContain("<strong>hello</strong>");

    expect((await anon.get("/user")).status).toBe(401);
    const userResponse = await admin.get("/user");
    expect(userResponse.status).toBe(200);
    expect((await json(userResponse)).login).toBe("admin");

    await seedIssue(admin, "e2e", "compat-user-export", "export me");
    expect((await anon.get("/user/export")).status).toBe(401);
    const exported = await admin.get("/user/export");
    expect(exported.status).toBe(200);
    const body = await json(exported);
    expect(body.schema_version).toBe(1);
    expect(body.repos.length).toBeGreaterThan(0);
  });

  test("compatible issues support auth, filters, validation, labels, close, and delete", async () => {
    const owner = "e2e";
    const repo = "compat-issues";
    expect((await anon.post(`/repos/${owner}/${repo}/issues`, { title: "no" })).status).toBe(401);
    const aliceNumber = await seedIssue(alice, owner, repo, "alice issue");
    await seedIssue(bob, owner, repo, "bob issue");

    expect((await alice.post(`/repos/${owner}/${repo}/issues`, { title: " " })).status).toBe(422);
    expect((await alice.post(`/repos/${owner}/${repo}/issues`, { title: "with labels", labels: ["enhancement"] })).status).toBe(201);
    expect((await bob.patch(`/repos/${owner}/${repo}/issues/${aliceNumber}`, { title: "bob edit" })).status).toBe(403);

    const patched = await alice.patch(`/repos/${owner}/${repo}/issues/${aliceNumber}`, {
      state: "closed",
      state_reason: "completed",
      labels: ["bug"]
    });
    expect(patched.status).toBe(200);
    expect((await json(patched)).closed_at).toBeTruthy();

    const byCreator = await anon.get(`/repos/${owner}/${repo}/issues?state=all&creator=alice`);
    expect((await json(byCreator)).some((item: any) => item.number === aliceNumber)).toBe(true);

    const byLabel = await anon.get(`/repos/${owner}/${repo}/issues?state=all&labels=bug`);
    expect((await json(byLabel)).some((item: any) => item.number === aliceNumber)).toBe(true);

    expect((await alice.patch(`/repos/${owner}/${repo}/issues/${aliceNumber}`, { state: "invalid-state" })).status).toBe(422);

    const deleteRepo = "compat-issues-delete";
    const deleteNumber = await seedIssue(admin, owner, deleteRepo, "delete me");
    expect((await admin.delete(`/api/v1/repos/${owner}/${deleteRepo}/threads/${deleteNumber}`)).status).toBe(204);
    expect((await anon.get(`/repos/${owner}/${deleteRepo}/issues/${deleteNumber}`)).status).toBe(404);
  });

  test("compatible comments paginate, update counts, validate, and enforce ownership", async () => {
    const owner = "e2e";
    const repo = "compat-comments";
    const number = await seedIssue(admin, owner, repo, "thread");
    const first = await seedComment(alice, owner, repo, number, "first");
    await seedComment(alice, owner, repo, number, "second");

    const list = await anon.get(`/repos/${owner}/${repo}/issues/${number}/comments?per_page=1&page=1`);
    expect(list.status).toBe(200);
    expect(list.headers.get("link")).toBeTruthy();
    expect((await json(await anon.get(`/repos/${owner}/${repo}/issues/${number}`))).comments).toBe(2);

    expect((await bob.patch(`/repos/${owner}/${repo}/issues/comments/${first}`, { body: "bob" })).status).toBe(403);
    expect((await alice.patch(`/repos/${owner}/${repo}/issues/comments/${first}`, { body: "alice edit" })).status).toBe(200);
    expect((await alice.patch(`/repos/${owner}/${repo}/issues/comments/${first}`, { body: "" })).status).toBe(422);
    expect((await alice.delete(`/repos/${owner}/${repo}/issues/comments/${first}`)).status).toBe(204);
    expect((await json(await anon.get(`/repos/${owner}/${repo}/issues/${number}`))).comments).toBe(1);
  });

  test("reactions are idempotent and exposed through compatible and native APIs", async () => {
    const owner = "e2e";
    const repo = "compat-reactions";
    const number = await seedIssue(admin, owner, repo, "thread");
    const commentId = await seedComment(alice, owner, repo, number, "comment");

    const first = await bob.post(`/repos/${owner}/${repo}/issues/comments/${commentId}/reactions`, { content: "+1" });
    expect(first.status).toBe(201);
    const reactionId = (await json(first)).id;
    expect((await bob.post(`/repos/${owner}/${repo}/issues/comments/${commentId}/reactions`, { content: "+1" })).status).toBe(200);
    expect((await alice.delete(`/repos/${owner}/${repo}/issues/comments/${commentId}/reactions/${reactionId}`)).status).toBe(403);
    expect((await bob.delete(`/repos/${owner}/${repo}/issues/comments/${commentId}/reactions/${reactionId}`)).status).toBe(204);

    expect((await alice.post(`/repos/${owner}/${repo}/issues/comments/${commentId}/reactions`, { content: "invalid" })).status).toBe(422);
    const native = await alice.post(`/api/v1/repos/${owner}/${repo}/comments/${commentId}/reactions`, { content: "heart" });
    expect(native.status).toBe(201);
    expect((await json(native)).heart).toBe(1);
    expect((await alice.delete(`/api/v1/repos/${owner}/${repo}/comments/${commentId}/reactions/heart`)).status).toBe(204);
  });

  test("search, labels, native threads, comments, admin, auth, and export", async () => {
    const owner = "e2e";
    const repo = "native-all";
    const n1 = await seedIssue(admin, owner, repo, "hello bug");
    await admin.patch(`/repos/${owner}/${repo}/issues/${n1}`, { labels: ["bug"] });

    const search = await anon.get(`/search/issues?q=hello%20repo:${owner}/${repo}%20label:bug`);
    expect(search.status).toBe(200);
    expect((await json(search)).items.length).toBeGreaterThan(0);

    expect((await alice.post(`/api/v1/repos/${owner}/${repo}/labels`, { name: "x" })).status).toBe(403);
    expect((await admin.post(`/api/v1/repos/${owner}/${repo}/labels`, { name: "x" })).status).toBe(201);
    expect((await anon.get(`/api/v1/repos/${owner}/${repo}/labels`)).status).toBe(200);

    const created = await alice.post(`/api/v1/repos/${owner}/${repo}/threads`, { title: "My Article", body: "hello", slug: "my-article" });
    expect(created.status).toBe(201);
    expect((await alice.post(`/api/v1/repos/${owner}/${repo}/threads`, { title: "dup", slug: "my-article" })).status).toBe(409);
    const lookup = await anon.get(`/api/v1/repos/${owner}/${repo}/threads?slug=my-article&state=all`);
    expect((await json(lookup)).data).toHaveLength(1);

    const comments = await alice.post(`/api/v1/repos/${owner}/${repo}/threads/${(await json(created)).number}/comments`, { body: "native comment" });
    expect(comments.status).toBe(201);
    expect((await anon.get(`/api/v1/repos/${owner}/${repo}/threads?limit=1&direction=asc&state=all`)).status).toBe(200);

    expect((await anon.get("/api/v1/auth/me")).status).toBe(401);
    expect((await admin.get("/api/v1/auth/me")).status).toBe(200);
    expect((await anon.get("/api/v1/auth/account/authorize?redirect_uri=https://app.jihuayu.com/done")).status).toBe(501);
    expect((await anon.post("/api/v1/auth/account", { id_token: "fake" })).status).toBe(501);
    expect((await anon.post("/api/v1/auth/google", { token: "fake" })).status).toBe(501);
    expect((await anon.post("/api/v1/auth/refresh")).status).toBe(401);

    expect((await alice.get(`/api/v1/repos/${owner}/${repo}/export`)).status).toBe(403);
    const exported = await admin.get(`/api/v1/repos/${owner}/${repo}/export?format=json`);
    expect(exported.status).toBe(200);
    expect((await json(exported)).threads.length).toBeGreaterThan(0);
    const csv = await admin.get(`/api/v1/repos/${owner}/${repo}/export?format=csv`);
    expect(csv.status).toBe(200);
    expect(await csv.text()).toContain("thread_number");
    expect((await admin.get(`/api/v1/repos/${owner}/${repo}/export?format=xml`)).status).toBe(400);
  });
});
