import { describe, expect, test } from "vitest";
import {
  accountCallbackUri,
  accountIssuer,
  buildAccountAuthorizeLocation,
  redirectWithUserState,
  verifyAccountOAuthState
} from "../../src/account-auth";
import type { AppContext } from "../../src/types";

const jwtSecret = new TextEncoder().encode("atrium-account-test-secret");

function ctx(overrides: Partial<AppContext["env"]> = {}): AppContext {
  return {
    db: {} as AppContext["db"],
    env: {
      DB: {} as D1Database,
      ACCOUNT_ISSUER: "https://account.jihuayu.com/",
      ACCOUNT_CLIENT_ID: "atrium",
      ACCOUNT_SCOPE: "openid profile email",
      ...overrides
    },
    baseUrl: "https://atrium.jihuayu.com",
    tokenCacheTtl: 3600,
    jwtSecret,
    statefulSessions: false
  };
}

describe("account auth helpers", () => {
  test("normalizes account issuer and default callback uri", () => {
    expect(accountIssuer(ctx().env)).toBe("https://account.jihuayu.com");
    expect(accountCallbackUri(ctx(), "account")).toBe("https://atrium.jihuayu.com/api/v1/auth/account/callback");
  });

  test("builds OIDC authorize URL with signed state", async () => {
    const location = await buildAccountAuthorizeLocation(ctx(), "account", "https://app.jihuayu.com/done", "client-state");
    const url = new URL(location);

    expect(`${url.origin}${url.pathname}`).toBe("https://account.jihuayu.com/oauth/authorize");
    expect(url.searchParams.get("response_type")).toBe("code");
    expect(url.searchParams.get("client_id")).toBe("atrium");
    expect(url.searchParams.get("redirect_uri")).toBe("https://atrium.jihuayu.com/api/v1/auth/account/callback");
    expect(url.searchParams.get("scope")).toBe("openid profile email");
    expect(url.searchParams.get("nonce")).toBeTruthy();

    const state = await verifyAccountOAuthState(ctx(), url.searchParams.get("state") ?? "");
    expect(state.redirectUri).toBe("https://app.jihuayu.com/done");
    expect(state.userState).toBe("client-state");
    expect(state.nonce).toBe(url.searchParams.get("nonce"));
  });

  test("adds user state to final redirect", () => {
    expect(redirectWithUserState("https://app.jihuayu.com/done?x=1", "state-1")).toBe("https://app.jihuayu.com/done?x=1&state=state-1");
  });
});
