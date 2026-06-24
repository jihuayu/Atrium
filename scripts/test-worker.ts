import { spawn, spawnSync } from "node:child_process";
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createServer } from "node:net";
import { setTimeout as sleep } from "node:timers/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { CompactEncrypt, exportJWK, generateKeyPair, type JWK } from "jose";

const root = dirname(dirname(fileURLToPath(import.meta.url)));
const workerDir = join(root, "deploy", "worker");
const port = process.env.ATRIUM_TEST_PORT ? Number(process.env.ATRIUM_TEST_PORT) : await findOpenPort(8788);
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
  const discovery = await discoveryTestConfig();
  writeFileSync(
    configPath,
    template
      .replaceAll("__TEST_DB_NAME__", "atrium-test-local")
      .replaceAll("__TEST_DB_ID__", "00000000-0000-0000-0000-000000000000")
      .replaceAll("__TEST_PORT__", String(port))
      .replaceAll("__TEST_BYPASS_SECRET__", bypassSecret)
      .replaceAll("__TEST_SUPER_ADMIN_ACCOUNT_IDS__", superAdminAccountIds)
      .replaceAll("__TEST_JWT_SECRET__", jwtSecret)
      .replaceAll("__TEST_DISCOVERY_PRIVATE_JWK__", tomlString(JSON.stringify(discovery.privateJwk)))
      .replaceAll("__TEST_DISCOVERY_PUBLIC_JWK__", tomlString(JSON.stringify(discovery.publicJwk)))
      .replaceAll("__TEST_DISCOVERY_KEY_ID__", tomlString(discovery.keyId))
      .replaceAll("__TEST_DISCOVERY_WELL_KNOWN__", tomlString(JSON.stringify(discovery.wellKnown)))
      .replaceAll("__TEST_DISCOVERY_DNS_TXT__", tomlString(JSON.stringify(discovery.dnsTxt)))
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
      if (response.ok && (await response.text()).includes("Atrium - native")) break;
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

async function discoveryTestConfig() {
  const keyId = "test-discovery-key";
  const { privateKey, publicKey } = await generateKeyPair("RSA-OAEP-256", { extractable: true, modulusLength: 2048 });
  const privateJwk = withDiscoveryJwkMetadata(await exportJWK(privateKey), keyId, "decrypt");
  const publicJwk = withDiscoveryJwkMetadata(await exportJWK(publicKey), keyId, "encrypt");
  const encrypt = async (value: unknown) =>
    `enc:jwe:${await new CompactEncrypt(new TextEncoder().encode(JSON.stringify(value)))
      .setProtectedHeader({ alg: "RSA-OAEP-256", enc: "A256GCM", kid: keyId })
      .encrypt(publicKey)}`;

  const filePlain = {
    atrium: "v1",
    name: "Discover File",
    admin_emails: ["owner@test.com"],
    contact_email: "owner@test.com"
  };
  const fileEncrypted = {
    atrium: "v1",
    origin: "https://discover-file-encrypted.example.com",
    name: "Discover File Encrypted",
    admin_emails: await encrypt(["owner@test.com"]),
    contact_email: await encrypt("owner@test.com")
  };
  const dnsPlain = {
    atrium: "v1",
    name: "Discover DNS",
    admin_emails: ["owner@test.com"]
  };
  const dnsEncrypted = {
    atrium: "v1",
    origin: "https://discover-dns-encrypted.example.com",
    name: "Discover DNS Encrypted",
    admin_emails: await encrypt(["owner@test.com"])
  };

  return {
    keyId,
    privateJwk,
    publicJwk,
    wellKnown: {
      "https://discover-file.example.com": filePlain,
      [fileEncrypted.origin]: fileEncrypted,
      "https://discover-mismatch.example.com": {
        atrium: "v1",
        origin: "https://other.example.com",
        name: "Mismatch",
        admin_emails: ["owner@test.com"]
      },
      "https://discover-bad-jwe.example.com": {
        atrium: "v1",
        origin: "https://discover-bad-jwe.example.com",
        name: "Bad JWE",
        admin_emails: "enc:jwe:not-a-jwe"
      },
      "https://discover-wrong-type.example.com": {
        atrium: "v1",
        origin: "https://discover-wrong-type.example.com",
        name: "Wrong Type",
        admin_emails: await encrypt("owner@test.com")
      },
      "https://discover-conflict.example.com": {
        atrium: "v1",
        name: "Conflict",
        admin_emails: ["owner@test.com"]
      }
    },
    dnsTxt: {
      "discover-dns.example.com": `atrium-site=${JSON.stringify(dnsPlain)}`,
      "discover-dns-encrypted.example.com": `atrium-site=${JSON.stringify(dnsEncrypted)}`
    }
  };
}

function withDiscoveryJwkMetadata(jwk: JWK, keyId: string, keyOp: "encrypt" | "decrypt"): JWK {
  return { ...jwk, kid: keyId, alg: "RSA-OAEP-256", key_ops: [keyOp] };
}

function tomlString(value: string): string {
  return JSON.stringify(value);
}

async function findOpenPort(start: number): Promise<number> {
  for (let portCandidate = start; portCandidate < start + 100; portCandidate += 1) {
    if (await canListen(portCandidate)) return portCandidate;
  }
  throw new Error(`No open port found starting at ${start}`);
}

function canListen(portCandidate: number): Promise<boolean> {
  return new Promise((resolve) => {
    const server = createServer();
    server.unref();
    server.once("error", () => resolve(false));
    server.listen(portCandidate, "127.0.0.1", () => {
      server.close(() => resolve(true));
    });
  });
}
