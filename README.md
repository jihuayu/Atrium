# Atrium

Atrium is a Cloudflare Worker comment backend that stores GitHub Issues-compatible comment data in Cloudflare D1.

It exposes two API surfaces:

- GitHub-compatible endpoints for clients that expect Issues, comments, labels, reactions, search, `/markdown`, `/user`, and `/user/export`.
- Native `/api/v1` endpoints for first-party comment UI flows, Jihuayu Account SSO cookie login, JWT auth, cursor pagination, repo settings, labels, reactions, and exports.

## Runtime

Atrium is Worker-only. The runtime entrypoint is [`src/index.ts`](src/index.ts), and persistent data lives in the D1 binding configured by [`wrangler.jsonc`](wrangler.jsonc).

Required binding:

- `DB`: Cloudflare D1 database

Important environment variables:

- `BASE_URL`: public API origin used when formatting API URLs and cookie security
- `TOKEN_CACHE_TTL`: GitHub/provider token cache TTL in seconds, default `3600`
- `JWT_SECRET`: HS256 secret for native access and refresh tokens
- `ACCOUNT_BASE_URL`: account service origin, default `https://account.jihuayu.com`
- `ACCOUNT_AUDIENCE`: account session introspection audience, default `atrium`
- `ACCOUNT_INTERNAL_SECRET`: optional Worker secret sent as `x-internal-secret` to the account introspection endpoint
- `ATRIUM_TEST_BYPASS_SECRET`: local/CI-only HTTP test bypass secret

Native login uses the parent-domain SSO cookie set by `account.jihuayu.com`:

- `GET /api/v1/auth/account/authorize?redirect_uri=...&state=...`
- `GET /api/v1/auth/account/callback`
- `POST /api/v1/auth/account` exchanges the active SSO cookie for Atrium access/refresh cookies

Atrium also accepts the account SSO cookie directly on Native `/api/v1` requests by calling `/internal/session/introspect` on the account service. The legacy `GET /api/v1/auth/github/authorize` and callback routes remain as compatibility aliases for the account login bridge. `POST /api/v1/auth/github` still accepts a GitHub token for clients that need direct GitHub-compatible token exchange.

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
pnpm db:migrate:local
```

Remote:

```bash
pnpm db:migrate:remote
```

## Deploy

```bash
pnpm deploy
```

Production uses the `atrium-db` D1 binding and the custom Worker domain `https://atrium.jihuayu.com`.
Deployment follows the same local Wrangler flow as `jihuayu-account`: run remote D1
migrations from a logged-in local Wrangler session, then run `pnpm deploy`.
