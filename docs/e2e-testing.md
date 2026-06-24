# Atrium Worker E2E Tests

Atrium uses HTTP black-box tests against the Cloudflare Worker runtime.

## Commands

```bash
pnpm test
pnpm test:worker
```

`pnpm test` runs pure unit tests. `pnpm test:worker` runs `scripts/test-worker.ts`, which:

1. Generates a temporary Wrangler config from `deploy/worker/wrangler.test.toml.template`.
2. Initializes local D1 with `deploy/worker/test_init.sql`.
3. Starts `wrangler dev` on a local port.
4. Runs `vitest run tests/worker` against the local Worker URL.
5. Cleans up the temporary config and local D1 state.

## Test Auth Bypass

Worker E2E tests use:

```text
Authorization: testuser {secret}:{id}:{login}:{email}
```

The bypass is only active when `ATRIUM_TEST_BYPASS_SECRET` or the legacy `XTALK_TEST_BYPASS_SECRET` is configured. Production Worker config should not set either variable.

## Coverage

The Worker E2E suite covers:

- GitHub-compatible issues, comments, reactions, labels, search, markdown, user, and user export flows.
- Native auth, Jihuayu Account SSO cookie login branches, admin settings, threads, comments, reactions, labels, exports, slug lookup, cursor pagination, and permission branches.
- D1-backed read/write behavior through Wrangler local D1.
