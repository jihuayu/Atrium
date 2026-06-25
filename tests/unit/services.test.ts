import { afterEach, describe, expect, test, vi } from "vitest";
import { resolveAccountCookieUser, resolveNativeRequestUser } from "../../src/services";
import type { AppContext } from "../../src/types";
import type { SqlValue } from "../../src/db";
import { signJwt, toPublicUser } from "../../src/utils";

type UserRow = {
  id: number;
  login: string;
  display_name: string;
  email: string;
  avatar_url: string;
  type: string;
  site_admin: number;
};

type IdentityRow = {
  user_id: number;
  provider: string;
  provider_user_id: string;
  email: string;
  avatar_url: string;
};

class FakeDb {
  users: UserRow[] = [
    {
      id: 1,
      login: "user",
      display_name: "Old Name",
      email: "old@example.com",
      avatar_url: "https://account.jihuayu.com/old.png",
      type: "User",
      site_admin: 0
    }
  ];
  identities: IdentityRow[] = [
    {
      user_id: 1,
      provider: "account",
      provider_user_id: "acct-1",
      email: "old@example.com",
      avatar_url: "https://account.jihuayu.com/old.png"
    }
  ];

  async first<T>(sql: string, params: SqlValue[] = []): Promise<T | null> {
    if (sql.includes("FROM user_identities ui JOIN users u")) {
      const [provider, providerUserId] = params;
      const identity = this.identities.find((item) => item.provider === provider && item.provider_user_id === providerUserId);
      const user = identity ? this.users.find((item) => item.id === identity.user_id) : null;
      return (user as T | undefined) ?? null;
    }

    if (sql === "SELECT id FROM users WHERE login = ?1") {
      const [login] = params;
      const user = this.users.find((item) => item.login === login);
      return (user ? ({ id: user.id } as T) : null) ?? null;
    }

    if (sql === "SELECT id, login, display_name, email, avatar_url, type, site_admin FROM users WHERE id = ?1") {
      const [id] = params;
      const user = this.users.find((item) => item.id === Number(id));
      return (user as T | undefined) ?? null;
    }

    throw new Error(`Unhandled first SQL: ${sql}`);
  }

  async execute(sql: string, params: SqlValue[] = []): Promise<number> {
    if (sql.startsWith("UPDATE users SET login")) {
      const [login, displayName, email, avatarUrl, type, id] = params;
      const user = this.users.find((item) => item.id === id);
      if (user) {
        user.login = String(login);
        user.display_name = String(displayName);
        user.email = String(email);
        user.avatar_url = String(avatarUrl);
        user.type = String(type);
        return 1;
      }
      return 0;
    }

    if (sql.startsWith("UPDATE user_identities SET email")) {
      const [email, avatarUrl, provider, providerUserId] = params;
      const identity = this.identities.find((item) => item.provider === provider && item.provider_user_id === providerUserId);
      if (identity) {
        identity.email = String(email);
        identity.avatar_url = String(avatarUrl);
        return 1;
      }
      return 0;
    }

    throw new Error(`Unhandled execute SQL: ${sql}`);
  }

  async all<T>(): Promise<T[]> {
    return [];
  }
}

function ctx(db: FakeDb): AppContext {
  return {
    db: db as unknown as AppContext["db"],
    env: {
      DB: {} as D1Database,
      ACCOUNT_BASE_URL: "https://account.jihuayu.com",
      ACCOUNT_AUDIENCE: "atrium",
      ACCOUNT_INTERNAL_SECRET: "internal-secret"
    },
    baseUrl: "https://atrium.jihuayu.com",
    jwtSecret: new TextEncoder().encode("atrium-account-test-secret"),
    statefulSessions: false
  };
}

describe("account user sync", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  test("refreshes cached account identity profile before issuing Atrium users", async () => {
    const db = new FakeDb();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json({
          active: true,
          user: {
            sub: "acct-1",
            handle: "jihuayu",
            displayName: "季华宇",
            avatarUrl: "https://account.jihuayu.com/avatar.png",
            email: "jihuayu@example.com"
          }
        })
      )
    );

    const user = await resolveAccountCookieUser(ctx(db), "__Secure-jihuayu_sso=session");

    expect(user).toMatchObject({
      id: 1,
      login: "account-acct-1",
      display_name: "季华宇",
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png",
      account_sub: "acct-1"
    });
    expect(toPublicUser(user!, true)).toMatchObject({
      login: "季华宇",
      display_name: "季华宇",
      email: "jihuayu@example.com"
    });
    expect(db.users[0]).toMatchObject({
      login: "account-acct-1",
      display_name: "季华宇",
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png"
    });
    expect(db.identities[0]).toMatchObject({
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png"
    });
  });

  test("uses account display name as the Atrium login and ignores username", async () => {
    const db = new FakeDb();
    vi.stubGlobal(
      "fetch",
      vi.fn(async () =>
        Response.json({
          active: true,
          user: {
            sub: "acct-1",
            username: "account-username",
            displayName: "Visible Name",
            avatar_url: "https://account.jihuayu.com/avatar-2.png",
            email: "jihuayu@example.com"
          }
        })
      )
    );

    const user = await resolveAccountCookieUser(ctx(db), "__Secure-jihuayu_sso=session");

    expect(user).toMatchObject({
      id: 1,
      login: "account-acct-1",
      display_name: "Visible Name",
      avatar_url: "https://account.jihuayu.com/avatar-2.png"
    });
    expect(toPublicUser(user!)).toMatchObject({
      login: "Visible Name",
      display_name: "Visible Name"
    });
    expect(db.users[0]).toMatchObject({
      login: "account-acct-1",
      display_name: "Visible Name",
      avatar_url: "https://account.jihuayu.com/avatar-2.png"
    });
  });

  test("prefers active account SSO over a stale Atrium access cookie", async () => {
    const db = new FakeDb();
    const appCtx = ctx(db);
    const staleAccessToken = await signJwt(
      {
        sub: "1",
        login: "old-login",
        token_type: "access"
      },
      appCtx.jwtSecret
    );
    const fetchMock = vi.fn(async () =>
      Response.json({
        active: true,
        user: {
          sub: "acct-1",
          username: "fresh-username",
          displayName: "Fresh Name",
          avatarUrl: "https://account.jihuayu.com/fresh.png",
          email: "fresh@example.com"
        }
      })
    );
    vi.stubGlobal("fetch", fetchMock);

    const user = await resolveNativeRequestUser(appCtx, undefined, `atrium_access=${staleAccessToken}; __Secure-jihuayu_sso=session`);

    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(user).toMatchObject({
      id: 1,
      login: "account-acct-1",
      display_name: "Fresh Name",
      email: "fresh@example.com"
    });
    expect(toPublicUser(user!)).toMatchObject({
      login: "Fresh Name",
      display_name: "Fresh Name"
    });
    expect(db.users[0]).toMatchObject({
      login: "account-acct-1",
      display_name: "Fresh Name",
      email: "fresh@example.com"
    });
  });
});
