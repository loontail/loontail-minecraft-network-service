import { describe, expect, it } from "vitest";

import { formatBytes, shortUuid } from "@/shared/lib/format";

describe("formatBytes", () => {
  it("renders a zero size as a real value, not a dash", () => {
    expect(formatBytes(0)).toBe("0 B");
  });

  it("keeps whole bytes unrounded below 1 KB", () => {
    expect(formatBytes(512)).toBe("512 B");
  });

  it("scales to KB with one decimal", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
  });

  it("scales past KB — a 3 MB texture is not 3072.0 KB", () => {
    expect(formatBytes(3 * 1024 ** 2)).toBe("3.0 MB");
  });

  it("drops the decimal at 10 units and above", () => {
    expect(formatBytes(12 * 1024 ** 3)).toBe("12 GB");
  });

  it("renders a dash for a missing size", () => {
    expect(formatBytes(null)).toBe("—");
    expect(formatBytes(undefined)).toBe("—");
  });
});

describe("shortUuid", () => {
  it("elides the middle of a long id", () => {
    expect(shortUuid("0123456789abcdef0123456789abcdef")).toBe("01234567…cdef");
  });

  it("leaves short values alone", () => {
    expect(shortUuid("abc")).toBe("abc");
  });
});
