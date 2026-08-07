import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { NetworkPage } from "@/pages/NetworkPage";
import type { AnalyticsOverview, TimeseriesResponse } from "@/shared/types";
import { renderWithProviders } from "@/test/renderWithProviders";

const OVERVIEW: AnalyticsOverview = {
  playingNow: 7,
  onlineInNetwork: 21,
  openWorlds: 3,
  activeRelays: 2,
  totalUsers: 142,
};

const EMPTY_SERIES: TimeseriesResponse = {
  metric: "bootstraps",
  window: "24h",
  series: [],
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
    if (url.includes("/admin/analytics/overview")) {
      return Promise.resolve(jsonResponse(OVERVIEW));
    }
    if (url.includes("/admin/analytics/timeseries")) {
      return Promise.resolve(jsonResponse(EMPTY_SERIES));
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`));
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

// Scope each assertion to the card carrying that label, so a value can only
// satisfy the card it belongs to — checking labels and values separately passes
// even when two cards read each other's metric.
function statCard(label: RegExp) {
  const card = screen.getByText(label).closest('[data-slot="card"]');
  expect(card).not.toBeNull();
  return within(card as HTMLElement);
}

describe("NetworkPage", () => {
  it("pairs every stat card with its own overview metric", async () => {
    mockApi();
    renderWithProviders(<NetworkPage />);

    expect(
      screen.getByRole("heading", { name: /network/i }),
    ).toBeInTheDocument();

    await waitFor(() => {
      expect(statCard(/playing now/i).getByText("7")).toBeInTheDocument();
    });
    expect(statCard(/online in network/i).getByText("21")).toBeInTheDocument();
    expect(statCard(/open worlds/i).getByText("3")).toBeInTheDocument();
    expect(statCard(/active relays/i).getByText("2")).toBeInTheDocument();
    expect(statCard(/total users/i).getByText("142")).toBeInTheDocument();
  });

  it("shows the empty chart state when the series has no data", async () => {
    mockApi();
    renderWithProviders(<NetworkPage />);

    await waitFor(() => {
      expect(screen.getByText(/no activity yet/i)).toBeInTheDocument();
    });
  });

  it("exposes the time-range selector with 24h selected by default", async () => {
    mockApi();
    renderWithProviders(<NetworkPage />);

    const selected = await screen.findByRole("radio", { checked: true });
    expect(selected).toHaveTextContent("24h");
  });
});
