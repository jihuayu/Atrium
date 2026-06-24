export interface Env {
  DB: D1Database;
  BASE_URL?: string;
  JWT_SECRET?: string;
  ACCOUNT_BASE_URL?: string;
  ACCOUNT_AUDIENCE?: string;
  ACCOUNT_INTERNAL_SECRET?: string;
  ACCOUNT_ISSUER?: string;
  ATRIUM_SUPER_ADMIN_ACCOUNT_IDS?: string;
  ATRIUM_DISCOVERY_PRIVATE_JWK?: string;
  ATRIUM_DISCOVERY_PUBLIC_JWK?: string;
  ATRIUM_DISCOVERY_KEY_ID?: string;
  ATRIUM_TEST_DISCOVERY_WELL_KNOWN?: string;
  ATRIUM_TEST_DISCOVERY_DNS_TXT?: string;
  ATRIUM_TEST_BYPASS_SECRET?: string;
  XTALK_TEST_BYPASS_SECRET?: string;
}

export interface AuthUser {
  id: number;
  login: string;
  email: string;
  avatar_url: string;
  type: string;
  account_sub?: string;
}

export interface PublicUser {
  id: number;
  login: string;
  avatar_url: string;
  email?: string;
}

export interface WebsiteRow {
  id: number;
  key: string;
  name: string;
  created_at: string;
  updated_at: string;
}

export interface PageRow {
  id: number;
  website_id: number;
  key: string;
  title: string;
  url: string;
  normalized_url: string;
  metadata: string | null;
  comment_count: number;
  created_at: string;
  updated_at: string;
}

export interface AppContext {
  db: import("./db").Database;
  env: Env;
  baseUrl: string;
  jwtSecret: Uint8Array;
  user?: AuthUser;
  statefulSessions: boolean;
}

export interface ReactionCounts {
  like: number;
  dislike: number;
  heart: number;
  laugh: number;
  hooray: number;
  confused: number;
  rocket: number;
  eyes: number;
  total: number;
}

export const EMPTY_REACTION_COUNTS: ReactionCounts = {
  like: 0,
  dislike: 0,
  heart: 0,
  laugh: 0,
  hooray: 0,
  confused: 0,
  rocket: 0,
  eyes: 0,
  total: 0
};
