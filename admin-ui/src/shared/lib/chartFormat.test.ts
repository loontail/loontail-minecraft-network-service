import { describe, expect, it } from "vitest";

import {
  formatBucketLabel,
  formatBucketTick,
  formatLatency,
  formatPercent,
} from "@/shared/lib/chartFormat";

const BUCKET = "2026-08-06T14:35:00.000Z";

describe("formatBucketTick", () => {
  it("renders a clock time in the 24h window", () => {
    const tick = formatBucketTick(BUCKET, "24h");
    expect(tick).toBe(
      new Date(BUCKET).toLocaleTimeString([], {
        hour: "2-digit",
        minute: "2-digit",
      }),
    );
  });

  it("renders a calendar day in the 7d window", () => {
    const tick = formatBucketTick(BUCKET, "7d");
    expect(tick).toBe(
      new Date(BUCKET).toLocaleDateString([], {
        month: "short",
        day: "numeric",
      }),
    );
  });
});

describe("formatBucketLabel", () => {
  it("includes the time in the 24h window", () => {
    expect(formatBucketLabel(BUCKET, "24h")).toBe(
      new Date(BUCKET).toLocaleString([], {
        month: "short",
        day: "numeric",
        hour: "2-digit",
        minute: "2-digit",
      }),
    );
  });

  it("includes the weekday in the 7d window", () => {
    expect(formatBucketLabel(BUCKET, "7d")).toBe(
      new Date(BUCKET).toLocaleDateString([], {
        weekday: "short",
        month: "short",
        day: "numeric",
      }),
    );
  });
});

describe("formatLatency", () => {
  it("floors sub-millisecond latencies", () => {
    expect(formatLatency(0.4)).toBe("<1 ms");
  });

  it("switches to seconds at 1000 ms", () => {
    expect(formatLatency(1500)).toBe("1.50 s");
  });

  it("rounds milliseconds", () => {
    expect(formatLatency(12.6)).toBe("13 ms");
  });

  it("renders a dash for a missing value", () => {
    expect(formatLatency(null)).toBe("—");
  });
});

describe("formatPercent", () => {
  it("collapses a tiny non-zero ratio", () => {
    expect(formatPercent(0.0005)).toBe("<0.1%");
  });

  it("keeps one decimal below 10%", () => {
    expect(formatPercent(0.042)).toBe("4.2%");
  });

  it("drops the decimal at 10% and above", () => {
    expect(formatPercent(0.5)).toBe("50%");
  });
});
