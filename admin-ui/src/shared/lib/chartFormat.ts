import type { TimeRange } from "@/shared/types";

export function formatBucketTick(value: string, range: TimeRange): string {
  const date = new Date(value);
  if (range === "24h") {
    return date.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  }
  return date.toLocaleDateString([], { month: "short", day: "numeric" });
}

export function formatBucketLabel(value: string, range: TimeRange): string {
  const date = new Date(value);
  if (range === "24h") {
    return date.toLocaleString([], {
      month: "short",
      day: "numeric",
      hour: "2-digit",
      minute: "2-digit",
    });
  }
  return date.toLocaleDateString([], {
    weekday: "short",
    month: "short",
    day: "numeric",
  });
}

export function formatLatency(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || Number.isNaN(ms)) {
    return "—";
  }
  if (ms < 1) {
    return "<1 ms";
  }
  if (ms >= 1000) {
    return `${(ms / 1000).toFixed(2)} s`;
  }
  return `${Math.round(ms)} ms`;
}

export function formatPercent(ratio: number | null | undefined): string {
  if (ratio === null || ratio === undefined || Number.isNaN(ratio)) {
    return "—";
  }
  const pct = ratio * 100;
  if (pct > 0 && pct < 0.1) {
    return "<0.1%";
  }
  return `${pct.toFixed(pct < 10 ? 1 : 0)}%`;
}
