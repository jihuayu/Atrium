# Atrium

Atrium is a Cloudflare Worker comment backend backed by Cloudflare D1.

The public model is:

```text
website -> page -> comment -> reply
```

It exposes a native `/api/v1` API for Jihuayu Account SSO login, website administration, page registration, comments, replies, reactions, moderation, and website-scoped user bans. The old GitHub Issues-compatible API has been removed.

## Runtime

Atrium is Worker-only. The runtime entrypoint is [`src/index.ts`](src/index.ts), and persistent data lives in the D1 binding configured by [`wrangler.jsonc`](wrangler.jsonc).

Required binding:

- `DB`: Cloudflare D1 database

Important environment variables:

- `BASE_URL`: public API origin used when formatting callbacks and cookie security
- `JWT_SECRET`: HS256 secret for native access and refresh tokens
- `ACCOUNT_BASE_URL`: account service origin, default `https://account.jihuayu.com`
- `ACCOUNT_AUDIENCE`: account session introspection audience, default `atrium`
- `ACCOUNT_INTERNAL_SECRET`: optional Worker secret sent as `x-internal-secret` to the account introspection endpoint
- `ATRIUM_SUPER_ADMIN_ACCOUNT_IDS`: comma-separated Jihuayu Account `sub` values that can create/configure all websites
- `ATRIUM_TEST_BYPASS_SECRET`: local/CI-only HTTP test bypass secret

Native login uses the parent-domain SSO cookie set by `account.jihuayu.com`:

- `GET /api/v1/auth/account/authorize?redirect_uri=...&state=...`
- `GET /api/v1/auth/account/callback`
- `POST /api/v1/auth/account` exchanges the active SSO cookie for Atrium access/refresh cookies

Atrium also accepts the account SSO cookie directly on Native `/api/v1` requests by calling `/internal/session/introspect` on the account service.

## API Shape

Super admins create websites and configure website origins. Each website has its own administrators:

- `POST /api/v1/websites`
- `GET /api/v1/websites`
- `GET /api/v1/websites/{websiteKey}`
- `PATCH /api/v1/websites/{websiteKey}`
- `GET|POST /api/v1/websites/{websiteKey}/admins`
- `DELETE /api/v1/websites/{websiteKey}/admins/{userId}`

Website admins register pages explicitly:

- `PUT /api/v1/websites/{websiteKey}/pages/{pageKey}`
- `GET /api/v1/websites/{websiteKey}/pages/{pageKey}`
- `GET /api/v1/websites/{websiteKey}/pages`

Comments and replies are page-scoped:

- `GET|POST /api/v1/websites/{websiteKey}/pages/{pageKey}/comments`
- `PATCH|DELETE /api/v1/websites/{websiteKey}/comments/{commentId}`
- `PUT|DELETE /api/v1/websites/{websiteKey}/comments/{commentId}/reactions/{content}`

For frontend widgets, quick mode resolves website and page from the HTTP `Referer` header after matching configured website origins:

- `GET|POST /api/v1/comments/current`
- `GET /api/v1/comments/current/replies?comment_id=...`
- `PUT|DELETE /api/v1/comments/current/{commentId}/reactions/{content}`

Quick mode may auto-create/update the page for a matched website origin. It never creates websites implicitly.

Website admins moderate comments and ban users within a website:

- `GET /api/v1/websites/{websiteKey}/admin/comments`
- `GET|POST /api/v1/websites/{websiteKey}/bans`
- `DELETE /api/v1/websites/{websiteKey}/bans/{userId}`

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
