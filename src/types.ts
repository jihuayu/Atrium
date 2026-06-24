export interface Env {
  DB: D1Database;
  BASE_URL?: string;
  TOKEN_CACHE_TTL?: string;
  JWT_SECRET?: string;
  ACCOUNT_ISSUER?: string;
  ACCOUNT_CLIENT_ID?: string;
  ACCOUNT_CLIENT_SECRET?: string;
  ACCOUNT_REDIRECT_URI?: string;
  ACCOUNT_SCOPE?: string;
  ATRIUM_TEST_BYPASS_SECRET?: string;
  XTALK_TEST_BYPASS_SECRET?: string;
}

export interface GitHubUser {
  id: number;
  login: string;
  email: string;
  avatar_url: string;
  type: string;
  site_admin: boolean;
}

export interface ApiUser {
  login: string;
  id: number;
  avatar_url: string;
  html_url: string;
  type: string;
}

export interface Label {
  id: number;
  name: string;
  color: string;
  description: string;
}

export interface Reactions {
  url: string;
  total_count: number;
  "+1": number;
  "-1": number;
  laugh: number;
  confused: number;
  heart: number;
  hooray: number;
  rocket: number;
  eyes: number;
}

export interface RepoRow {
  id: number;
  owner: string;
  name: string;
  owner_user_id: number | null;
  admin_user_id: number | null;
  issue_counter: number;
}

export interface AppContext {
  db: import("./db").Database;
  env: Env;
  baseUrl: string;
  tokenCacheTtl: number;
  jwtSecret: Uint8Array;
  user?: GitHubUser;
  statefulSessions: boolean;
}

export interface IssueResponse {
  id: number;
  node_id: string;
  number: number;
  title: string;
  slug?: string | null;
  body?: string | null;
  body_html?: string | null;
  state: string;
  locked: boolean;
  user: ApiUser;
  labels: Label[];
  comments: number;
  created_at: string;
  updated_at: string;
  closed_at?: string | null;
  author_association: string;
  reactions: Reactions;
  url: string;
  html_url: string;
  comments_url: string;
}

export interface CommentResponse {
  id: number;
  node_id: string;
  body?: string | null;
  body_html?: string | null;
  user: ApiUser;
  created_at: string;
  updated_at: string;
  html_url: string;
  issue_url: string;
  author_association: string;
  reactions: Reactions;
}

export interface ReactionCounts {
  plus_one: number;
  minus_one: number;
  laugh: number;
  confused: number;
  heart: number;
  hooray: number;
  rocket: number;
  eyes: number;
  total: number;
}

export const EMPTY_REACTION_COUNTS: ReactionCounts = {
  plus_one: 0,
  minus_one: 0,
  laugh: 0,
  confused: 0,
  heart: 0,
  hooray: 0,
  rocket: 0,
  eyes: 0,
  total: 0
};
