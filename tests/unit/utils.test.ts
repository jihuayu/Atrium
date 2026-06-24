import { describe, expect, test } from "vitest";
import {
  buildSetCookie,
  clearCookie,
  cookieValue,
  decodeCursor,
  encodeCursor,
  parseSecret,
  parseToken,
  renderMarkdown
} from "../../src/utils";
import { ApiError } from "../../src/error";

describe("utils", () => {
  test("parses auth headers", () => {
    expect(parseToken("token abc")).toBe("abc");
    expect(parseToken("Bearer xyz")).toBe("xyz");
    expect(parseToken("bad xyz")).toBeNull();
  });

  test("encodes cursors as base64url json", () => {
    const cursor = encodeCursor(42);
    expect(decodeCursor(cursor)).toBe(42);
    expect(() => decodeCursor("not-valid")).toThrow(ApiError);
  });

  test("cookies match worker transport shape", () => {
    expect(buildSetCookie("atrium_access", "tok", 60, true)).toContain("HttpOnly");
    expect(buildSetCookie("atrium_access", "tok", 60, true)).toContain("Secure");
    expect(buildSetCookie("atrium_access", "tok", 60, true)).toContain("SameSite=None");
    expect(buildSetCookie("atrium_access", "tok", 60, false)).toContain("SameSite=Lax");
    expect(clearCookie("atrium_access", true)).toContain("SameSite=None");
    expect(clearCookie("atrium_access", false)).toContain("Max-Age=0");
    expect(cookieValue("a=1; atrium_access=tok", "atrium_access")).toBe("tok");
  });

  test("markdown escapes raw html", () => {
    expect(renderMarkdown("**hello**")).toContain("<strong>hello</strong>");
    expect(renderMarkdown("<script>alert(1)</script>")).not.toContain("<script>");
  });

  test("secret parser supports plain and encoded secrets", () => {
    expect(new TextDecoder().decode(parseSecret("not@base64"))).toBe("not@base64");
    expect(new TextDecoder().decode(parseSecret("YXRyaXVtLXNlY3JldA"))).toBe("atrium-secret");
  });
});
