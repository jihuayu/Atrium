import { CompactEncrypt, exportJWK, generateKeyPair, type JWK } from "jose";
import { describe, expect, test } from "vitest";
import {
  DISCOVERY_JWE_ALG,
  DISCOVERY_JWE_ENC,
  ENCRYPTED_FIELD_PREFIX,
  DiscoveryDocumentError,
  decryptFlatDiscoveryFields,
  isEncryptedDiscoveryValue,
  parseAtriumTxtPayloadsFromDoh,
  parseDiscoveryDocument,
  parseDnsTxtData
} from "../../src/discovery";
import type { AppContext } from "../../src/types";

const textEncoder = new TextEncoder();

describe("discovery", () => {
  test("parses flat plaintext discovery metadata", async () => {
    const metadata = await parseDiscoveryDocument(
      ctx(),
      {
        atrium: "v1",
        origin: "https://blog.example.com",
        name: "Blog",
        admin_emails: ["OWNER@example.com", "owner@example.com"],
        contact_email: "Contact@example.com"
      },
      "https://blog.example.com",
      "well-known"
    );

    expect(metadata).toEqual({
      origin: "https://blog.example.com",
      websiteKey: "blog.example.com",
      name: "Blog",
      adminEmails: ["owner@example.com"],
      contactEmail: "contact@example.com",
      source: "well-known"
    });
  });

  test("derives origin and website key from the referer origin when origin is omitted", async () => {
    const metadata = await parseDiscoveryDocument(
      ctx(),
      {
        atrium: "v1",
        name: "Blog",
        admin_emails: ["owner@example.com"]
      },
      "https://blog.example.com",
      "dns-txt"
    );

    expect(metadata.origin).toBe("https://blog.example.com");
    expect(metadata.websiteKey).toBe("blog.example.com");
    expect(metadata.source).toBe("dns-txt");
  });

  test("detects and decrypts enc:jwe flat fields", async () => {
    const fixture = await jweFixture();
    const encryptedEmails = await fixture.encrypt(["owner@example.com"]);
    const document = await decryptFlatDiscoveryFields(ctx(fixture), {
      atrium: "v1",
      admin_emails: encryptedEmails,
      name: "Blog"
    });

    expect(isEncryptedDiscoveryValue(encryptedEmails)).toBe(true);
    expect(isEncryptedDiscoveryValue("plain")).toBe(false);
    expect(document.admin_emails).toEqual(["owner@example.com"]);
  });

  test("validates decrypted field types", async () => {
    const fixture = await jweFixture();
    await expect(
      parseDiscoveryDocument(
        ctx(fixture),
        {
          atrium: "v1",
          origin: "https://blog.example.com",
          name: "Blog",
          admin_emails: await fixture.encrypt("owner@example.com")
        },
        "https://blog.example.com",
        "well-known"
      )
    ).rejects.toThrow(DiscoveryDocumentError);
  });

  test("rejects origin mismatch and invalid jwe", async () => {
    await expect(
      parseDiscoveryDocument(
        ctx(),
        {
          atrium: "v1",
          origin: "https://other.example.com",
          name: "Blog",
          admin_emails: ["owner@example.com"]
        },
        "https://blog.example.com",
        "well-known"
      )
    ).rejects.toThrow(DiscoveryDocumentError);

    await expect(
      parseDiscoveryDocument(
        ctx(),
        {
          atrium: "v1",
          origin: "https://blog.example.com",
          name: "Blog",
          admin_emails: `${ENCRYPTED_FIELD_PREFIX}not-a-jwe`
        },
        "https://blog.example.com",
        "well-known"
      )
    ).rejects.toThrow(DiscoveryDocumentError);
  });

  test("rejects declared website_key in discovery metadata", async () => {
    await expect(
      parseDiscoveryDocument(
        ctx(),
        {
          atrium: "v1",
          website_key: "blog.example.com",
          name: "Blog",
          admin_emails: ["owner@example.com"]
        },
        "https://blog.example.com",
        "well-known"
      )
    ).rejects.toThrow(DiscoveryDocumentError);
  });

  test("parses TXT records and concatenates multi-string answers without spaces", () => {
    expect(parseDnsTxtData('"atrium-site={\\"atrium\\":" "\\"v1\\"}"').join("")).toBe('atrium-site={"atrium":"v1"}');

    const payloads = parseAtriumTxtPayloadsFromDoh({
      Answer: [
        { type: 16, data: '"not-atrium"' },
        { type: 16, data: '"atrium-site={\\"atrium\\":" "\\"v1\\",\\"origin\\":\\"https://blog.example.com\\"}"' }
      ]
    });

    expect(payloads).toEqual(['{"atrium":"v1","origin":"https://blog.example.com"}']);
  });
});

interface JweFixture {
  privateJwk: JWK;
  publicJwk: JWK;
  keyId: string;
  encrypt(value: unknown): Promise<string>;
}

function ctx(fixture?: JweFixture): AppContext {
  return {
    db: {} as AppContext["db"],
    env: {
      DB: {} as D1Database,
      ATRIUM_DISCOVERY_PRIVATE_JWK: fixture ? JSON.stringify(fixture.privateJwk) : undefined,
      ATRIUM_DISCOVERY_PUBLIC_JWK: fixture ? JSON.stringify(fixture.publicJwk) : undefined,
      ATRIUM_DISCOVERY_KEY_ID: fixture?.keyId
    },
    baseUrl: "http://127.0.0.1:8787",
    jwtSecret: new Uint8Array(),
    statefulSessions: false
  };
}

async function jweFixture(): Promise<JweFixture> {
  const keyId = "unit-key";
  const { privateKey, publicKey } = await generateKeyPair(DISCOVERY_JWE_ALG, { extractable: true, modulusLength: 2048 });
  const privateJwk = { ...(await exportJWK(privateKey)), kid: keyId, alg: DISCOVERY_JWE_ALG, key_ops: ["decrypt"] };
  const publicJwk = { ...(await exportJWK(publicKey)), kid: keyId, alg: DISCOVERY_JWE_ALG, key_ops: ["encrypt"] };
  return {
    privateJwk,
    publicJwk,
    keyId,
    encrypt: async (value) =>
      `${ENCRYPTED_FIELD_PREFIX}${await new CompactEncrypt(textEncoder.encode(JSON.stringify(value)))
        .setProtectedHeader({ alg: DISCOVERY_JWE_ALG, enc: DISCOVERY_JWE_ENC, kid: keyId })
        .encrypt(publicKey)}`
  };
}
