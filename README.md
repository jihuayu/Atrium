# Atrium

Atrium is a Cloudflare Worker comment backend that stores GitHub Issues-compatible comment data in Cloudflare D1.

It exposes two API surfaces:

- GitHub-compatible endpoints for clients that expect Issues, comments, labels, reactions, search, `/markdown`, `/user`, and `/user/export`.
- Native `/api/v1` endpoints for first-party comment UI flows, Jihuayu Account login, JWT auth, cursor pagination, repo settings, labels, reactions, and exports.

## Runtime

Atrium is Worker-only. The runtime entrypoint is [`src/index.ts`](src/index.ts), and persistent data lives in the D1 binding configured by [`deploy/worker/wrangler.toml`](deploy/worker/wrangler.toml).

Required binding:

- `DB`: Cloudflare D1 database

Important environment variables:

- `BASE_URL`: public API origin used when formatting API URLs and cookie security
- `TOKEN_CACHE_TTL`: GitHub/provider token cache TTL in seconds, default `3600`
- `JWT_SECRET`: HS256 secret for native access and refresh tokens
- `ACCOUNT_ISSUER`: OIDC issuer, default `https://account.jihuayu.com`
- `ACCOUNT_CLIENT_ID`: OAuth client id registered in `jihuayu-account`
- `ACCOUNT_CLIENT_SECRET`: optional OAuth client secret for confidential account clients
- `ACCOUNT_REDIRECT_URI`: optional registered callback override; defaults to `${BASE_URL}/api/v1/auth/account/callback`
- `ACCOUNT_SCOPE`: OIDC scopes, default `openid profile email`
- `ATRIUM_TEST_BYPASS_SECRET`: local/CI-only HTTP test bypass secret

Native login uses `account.jihuayu.com` as the OIDC provider:

- `GET /api/v1/auth/account/authorize?redirect_uri=...&state=...`
- `GET /api/v1/auth/account/callback`
- `POST /api/v1/auth/account` with `{ "id_token": "..." }`

The legacy `GET /api/v1/auth/github/authorize` and callback routes remain as compatibility aliases for the account login bridge. `POST /api/v1/auth/github` still accepts a GitHub token for clients that need direct GitHub-compatible token exchange.

## Development

```bash
pnpm install
pnpm typecheck
pnpm test
pnpm test:worker
```

`pnpm test:worker` starts `wrangler dev`, initializes a local D1 database from `deploy/worker/test_init.sql`, and runs HTTP black-box tests against the Worker.

## D1 Migrations

Local:

```bash
pnpm exec wrangler d1 migrations apply DB --config deploy/worker/wrangler.toml --local
```

Remote:

```bash
pnpm exec wrangler d1 migrations apply DB --config deploy/worker/wrangler.toml --remote
```

## Deploy

```bash
pnpm deploy
```

The Worker config keeps the existing production routes and D1 database binding.
