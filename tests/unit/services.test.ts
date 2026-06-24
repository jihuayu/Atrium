import { afterEach, describe, expect, test, vi } from "vitest";
import { resolveAccountCookieUser } from "../../src/services";
import type { AppContext } from "../../src/types";
import type { SqlValue } from "../../src/db";

type UserRow = {
  id: number;
  login: string;
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

    throw new Error(`Unhandled first SQL: ${sql}`);
  }

  async execute(sql: string, params: SqlValue[] = []): Promise<number> {
    if (sql.startsWith("UPDATE users SET login")) {
      const [login, email, avatarUrl, type, id] = params;
      const user = this.users.find((item) => item.id === id);
      if (user) {
        user.login = String(login);
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
      login: "jihuayu",
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png",
      account_sub: "acct-1"
    });
    expect(db.users[0]).toMatchObject({
      login: "jihuayu",
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png"
    });
    expect(db.identities[0]).toMatchObject({
      email: "jihuayu@example.com",
      avatar_url: "https://account.jihuayu.com/avatar.png"
    });
  });
});
