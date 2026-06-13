import { screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { LogsTrafficPage } from "@/pages/LogsTrafficPage";
import type {
  LogsTailResponse,
  RequestsSummary,
  RequestsTimeseriesResponse,
} from "@/shared/types";
import { renderWithProviders } from "@/test/renderWithProviders";

const SUMMARY: RequestsSummary = {
  window: "24h",
  totalRequests: 1234,
  errorRate: 0.025,
  avgLatencyMs: 42,
  statusMix: [
    { statusClass: "2xx", count: 1100 },
    { statusClass: "3xx", count: 20 },
    { statusClass: "4xx", count: 90 },
    { statusClass: "5xx", count: 24 },
  ],
  topPaths: [
    { path: "/api/catalog/builds", count: 540, avgLatencyMs: 31 },
    { path: "/admin/users", count: 120, avgLatencyMs: 88 },
  ],
};

const TIMESERIES: RequestsTimeseriesResponse = {
  window: "24h",
  metric: "requests",
  series: [
    { bucket: "2026-06-14T10:00:00Z", value: 120 },
    { bucket: "2026-06-14T11:00:00Z", value: 200 },
  ],
};

const TAIL: LogsTailResponse = {
  entries: [
    {
      ts: new Date().toISOString(),
      method: "GET",
      path: "/api/catalog/builds",
      status: 200,
      latencyMs: 12,
      userId: "abcd1234-5678-90ab-cdef-000000000000",
      authKind: "session",
      ip: "10.0.0.5",
      userAgent: "vitest",
      bytesOut: 2048,
    },
    {
      ts: new Date().toISOString(),
      method: "POST",
      path: "/admin/users",
      status: 503,
      latencyMs: 410,
      userId: null,
      authKind: "anon",
      ip: null,
      userAgent: null,
      bytesOut: null,
    },
  ],
};

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

const originalFetch = globalThis.fetch;

function mockApi() {
  // why: assign fetch directly rather than vi.stubGlobal so the shared
  // ResizeObserver stub from test/setup.ts survives this file's teardown
  // (vi.unstubAllGlobals would otherwise strip it and break Recharts).
  globalThis.fetch = vi.fn((input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/admin/logs/tail")) {
      return Promise.resolve(jsonResponse(TAIL));
    }
    if (url.includes("/admin/analytics/requests/summary")) {
      return Promise.resolve(jsonResponse(SUMMARY));
    }
    if (url.includes("/admin/analytics/requests/timeseries")) {
      return Promise.resolve(jsonResponse(TIMESERIES));
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`));
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe("LogsTrafficPage", () => {
  it("renders the heading, KPI cards, and traffic sections", async () => {
    mockApi();
    renderWithProviders(<LogsTrafficPage />);

    expect(
      screen.getByRole("heading", { name: /logs & traffic/i }),
    ).toBeInTheDocument();

    expect(screen.getByText(/total requests/i)).toBeInTheDocument();
    expect(screen.getByText(/error rate/i)).toBeInTheDocument();
    expect(screen.getByText(/avg latency/i)).toBeInTheDocument();

    // KPI values resolve from the summary endpoint.
    await waitFor(() => {
      expect(screen.getByText("1,234")).toBeInTheDocument();
    });
    expect(screen.getByText("2.5%")).toBeInTheDocument();
    expect(screen.getByText("42 ms")).toBeInTheDocument();
  });

  it("renders the top-paths table from summary data", async () => {
    mockApi();
    renderWithProviders(<LogsTrafficPage />);

    await waitFor(() => {
      expect(screen.getByText("/api/catalog/builds")).toBeInTheDocument();
    });
    expect(screen.getByText("540")).toBeInTheDocument();
  });

  it("renders the live backend logs table fed by the tail endpoint", async () => {
    mockApi();
    renderWithProviders(<LogsTrafficPage />);

    expect(screen.getByText(/backend logs/i)).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("/admin/users")).toBeInTheDocument();
    });
    // Status badges from the tail rows.
    expect(screen.getByText("200")).toBeInTheDocument();
    expect(screen.getByText("503")).toBeInTheDocument();
    // Method column.
    expect(screen.getByText("POST")).toBeInTheDocument();
  });
});
