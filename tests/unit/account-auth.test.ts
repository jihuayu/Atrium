import { afterEach, describe, expect, test, vi } from "vitest";
import { accountBaseUrl, accountLoginLocation, introspectAccountCookie, redirectWithUserState } from "../../src/account-auth";
import type { AppContext } from "../../src/types";

const jwtSecret = new TextEncoder().encode("atrium-account-test-secret");

function ctx(overrides: Partial<AppContext["env"]> = {}): AppContext {
  return {
    db: {} as AppContext["db"],
    env: {
      DB: {} as D1Database,
      ACCOUNT_BASE_URL: "https://account.jihuayu.com/",
      ACCOUNT_AUDIENCE: "atrium",
      ACCOUNT_INTERNAL_SECRET: "internal-secret",
      ...overrides
    },
    baseUrl: "https://atrium.jihuayu.com",
    tokenCacheTtl: 3600,
    jwtSecret,
    statefulSessions: false
  };
}

describe("account auth helpers", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  test("builds account login URL with return_to", () => {
    expect(accountBaseUrl(ctx().env)).toBe("https://account.jihuayu.com");
    expect(accountLoginLocation(ctx().env, "https://atrium.jihuayu.com/api/v1/auth/account/callback")).toBe(
      "https://account.jihuayu.com/login?return_to=https%3A%2F%2Fatrium.jihuayu.com%2Fapi%2Fv1%2Fauth%2Faccount%2Fcallback"
    );
  });

  test("adds user state to final redirect", () => {
    expect(redirectWithUserState("https://app.jihuayu.com/done?x=1", "state-1")).toBe("https://app.jihuayu.com/done?x=1&state=state-1");
  });

  test("skips introspection when SSO cookie is absent", async () => {
    const fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);

    await expect(introspectAccountCookie(ctx(), "atrium_access=token")).resolves.toBeNull();
    expect(fetchMock).not.toHaveBeenCalled();
  });

  test("introspects account SSO cookie with internal secret", async () => {
    const fetchMock = vi.fn(async (_url: string, init: RequestInit) => {
      expect(_url).toBe("https://account.jihuayu.com/internal/session/introspect");
      expect((init.headers as Headers).get("x-internal-secret")).toBe("internal-secret");
      expect((init.headers as Headers).get("cookie")).toContain("__Secure-jihuayu_sso=session");
      expect(JSON.parse(String(init.body))).toEqual({ audience: "atrium" });
      return Response.json({
        active: true,
        user: {
          sub: "user-1",
          handle: "alice",
          displayName: "Alice",
          avatarUrl: "https://account.jihuayu.com/a.png",
          email: "alice@jihuayu.com"
        }
      });
    });
    vi.stubGlobal("fetch", fetchMock);

    await expect(introspectAccountCookie(ctx(), "__Secure-jihuayu_sso=session")).resolves.toEqual({
      sub: "user-1",
      handle: "alice",
      displayName: "Alice",
      avatarUrl: "https://account.jihuayu.com/a.png",
      email: "alice@jihuayu.com"
    });
  });
});
