import { ApiError } from "./error";

export type SqlValue = string | number | boolean | null;

interface D1Like {
  prepare(sql: string): D1PreparedStatement;
  batch<T = unknown>(statements: D1PreparedStatement[]): Promise<D1Result<T>[]>;
}

export class Database {
  private readonly db: D1Like;

  constructor(db: D1Database) {
    const anyDb = db as unknown as D1Like & {
      withSession?: (constraint: string) => D1Like;
    };
    this.db = anyDb.withSession ? anyDb.withSession("first-primary") : anyDb;
  }

  async execute(sql: string, params: SqlValue[] = []): Promise<number> {
    const result = await this.db.prepare(sql).bind(...params).run();
    if (!result.success) {
      throw ApiError.internal(result.error ?? "d1 execute failed");
    }
    return Number(result.meta?.changes ?? 0);
  }

  async first<T>(sql: string, params: SqlValue[] = []): Promise<T | null> {
    return (await this.db.prepare(sql).bind(...params).first<T>()) ?? null;
  }

  async all<T>(sql: string, params: SqlValue[] = []): Promise<T[]> {
    const result = await this.db.prepare(sql).bind(...params).all<T>();
    if (!result.success) {
      throw ApiError.internal(result.error ?? "d1 query failed");
    }
    return (result.results ?? []) as T[];
  }

  async batch(stmts: Array<[string, SqlValue[]]>): Promise<void> {
    const prepared = stmts.map(([sql, params]) => this.db.prepare(sql).bind(...params));
    const results = await this.db.batch(prepared);
    for (const result of results) {
      if (!result.success) {
        throw ApiError.internal(result.error ?? "d1 batch statement failed");
      }
    }
  }
}
