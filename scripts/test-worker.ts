import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { setTimeout as sleep } from "node:timers/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const workerDir = join(root, "deploy", "worker");
const port = Number(process.env.ATRIUM_TEST_PORT ?? 8788);
const bypassSecret = randomHex(16);
const jwtSecret = crypto.randomUUID() + crypto.randomUUID();
const superAdminAccountIds = "super@test.com";
const configPath = join(workerDir, `.wrangler.test.${process.pid}.toml`);

function run(command: string, args: string[], env?: NodeJS.ProcessEnv, cwd = workerDir) {
  console.log(`$ ${command} ${args.join(" ")}`);
  const result = spawnSync(command, args, {
    cwd,
    stdio: "inherit",
    env: { ...process.env, ...env }
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}

function cleanup() {
  if (existsSync(configPath)) rmSync(configPath);
  rmSync(join(workerDir, ".wrangler"), { recursive: true, force: true });
}

try {
  cleanup();
  const template = readFileSync(join(workerDir, "wrangler.test.toml.template"), "utf8");
  writeFileSync(
    configPath,
    template
      .replaceAll("__TEST_DB_NAME__", "atrium-test-local")
      .replaceAll("__TEST_DB_ID__", "00000000-0000-0000-0000-000000000000")
      .replaceAll("__TEST_PORT__", String(port))
      .replaceAll("__TEST_BYPASS_SECRET__", bypassSecret)
      .replaceAll("__TEST_SUPER_ADMIN_ACCOUNT_IDS__", superAdminAccountIds)
      .replaceAll("__TEST_JWT_SECRET__", jwtSecret)
  );

  run("pnpm", [
    "exec",
    "wrangler",
    "d1",
    "execute",
    "DB",
    "--config",
    configPath,
    "--file",
    join(workerDir, "test_init.sql"),
    "--local",
    "--yes"
  ]);

  const dev = spawn("pnpm", ["exec", "wrangler", "dev", "--config", configPath, "--port", String(port)], {
    cwd: workerDir,
    stdio: "inherit",
    env: process.env
  });

  const stop = () => {
    dev.kill("SIGTERM");
    cleanup();
  };
  process.on("SIGINT", stop);
  process.on("SIGTERM", stop);

  for (let i = 0; i < 90; i += 1) {
    if (dev.exitCode !== null) process.exit(dev.exitCode ?? 1);
    try {
      const response = await fetch(`http://127.0.0.1:${port}/`);
      if (response.ok) break;
    } catch {
      await sleep(1000);
    }
    if (i === 89) throw new Error("wrangler dev did not become ready");
  }

  run(
    "pnpm",
    ["exec", "vitest", "run", "tests/worker"],
    {
      ATRIUM_TEST_BASE_URL: `http://127.0.0.1:${port}`,
      ATRIUM_TEST_BYPASS_SECRET: bypassSecret
    },
    root
  );
  stop();
} catch (error) {
  cleanup();
  throw error;
}

function randomHex(bytes: number): string {
  const data = new Uint8Array(bytes);
  crypto.getRandomValues(data);
  return [...data].map((b) => b.toString(16).padStart(2, "0")).join("");
}
