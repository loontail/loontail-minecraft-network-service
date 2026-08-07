import { describe, expect, it, vi } from "vitest";

import { ApiError } from "@/shared/api/client";
import {
  batchUploadSummary,
  uploadSequentially,
} from "@/shared/lib/batchUpload";

function file(name: string): File {
  return new File(["x"], name, { type: "image/png" });
}

describe("uploadSequentially", () => {
  it("sends one file at a time", async () => {
    const inFlight: string[] = [];
    const send = vi.fn(async (f: File) => {
      inFlight.push(f.name);
      expect(inFlight).toHaveLength(1);
      await Promise.resolve();
      inFlight.pop();
    });

    const outcome = await uploadSequentially(
      [file("a.png"), file("b.png")],
      send,
    );

    expect(send).toHaveBeenCalledTimes(2);
    expect(outcome).toEqual({ ok: 2, failures: [], notAttempted: [] });
  });

  it("keeps the filename and the server's reason for each rejection", async () => {
    const outcome = await uploadSequentially(
      [file("small.png"), file("big.png")],
      (f) =>
        f.name === "big.png"
          ? Promise.reject(new ApiError(413, "file exceeds 32MB"))
          : Promise.resolve(),
    );

    expect(outcome.ok).toBe(1);
    expect(outcome.failures).toEqual([
      { name: "big.png", message: "file exceeds 32MB" },
    ]);
    expect(outcome.notAttempted).toEqual([]);
  });

  it("stops the run on an expired session instead of firing doomed uploads", async () => {
    const send = vi.fn((f: File) =>
      f.name === "one.png"
        ? Promise.resolve()
        : Promise.reject(new ApiError(401, "session expired")),
    );

    const outcome = await uploadSequentially(
      [file("one.png"), file("two.png"), file("three.png"), file("four.png")],
      send,
    );

    expect(send).toHaveBeenCalledTimes(2);
    expect(outcome.ok).toBe(1);
    expect(outcome.failures).toEqual([
      { name: "two.png", message: "session expired" },
    ]);
    expect(outcome.notAttempted).toEqual(["three.png", "four.png"]);
  });

  it("keeps going after a per-file rejection", async () => {
    const send = vi.fn((f: File) =>
      f.name === "two.png"
        ? Promise.reject(new ApiError(415, "unsupported type"))
        : Promise.resolve(),
    );

    const outcome = await uploadSequentially(
      [file("one.png"), file("two.png"), file("three.png")],
      send,
    );

    expect(send).toHaveBeenCalledTimes(3);
    expect(outcome.ok).toBe(2);
    expect(outcome.notAttempted).toEqual([]);
  });
});

describe("batchUploadSummary", () => {
  it("names the offending file and its reason on a partial failure", () => {
    expect(
      batchUploadSummary(
        {
          ok: 4,
          failures: [{ name: "big.png", message: "file exceeds 32MB" }],
          notAttempted: [],
        },
        "screenshot",
      ),
    ).toBe("Uploaded 4, failed 1: big.png — file exceeds 32MB");
  });

  it("reports the reason when everything failed", () => {
    expect(
      batchUploadSummary(
        {
          ok: 0,
          failures: [
            { name: "a.png", message: "boom" },
            { name: "b.png", message: "boom" },
          ],
          notAttempted: [],
        },
        "screenshot",
      ),
    ).toBe("Failed to upload 2 screenshots: a.png — boom");
  });

  it("says how many files were never attempted", () => {
    expect(
      batchUploadSummary(
        {
          ok: 1,
          failures: [{ name: "two.png", message: "session expired" }],
          notAttempted: ["three.png", "four.png"],
        },
        "screenshot",
      ),
    ).toBe(
      "Uploaded 1, failed 1: two.png — session expired (2 not attempted)",
    );
  });

  it("pluralises the all-good case", () => {
    expect(
      batchUploadSummary({ ok: 1, failures: [], notAttempted: [] }, "file"),
    ).toBe("Uploaded 1 file");
    expect(
      batchUploadSummary({ ok: 3, failures: [], notAttempted: [] }, "file"),
    ).toBe("Uploaded 3 files");
  });
});
