import { screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { AnalyticsOverview, TimeseriesResponse } from "@/shared/types";
import { DashboardPage } from "@/pages/DashboardPage";
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

function mockApi() {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/admin/analytics/overview")) {
        return Promise.resolve(jsonResponse(OVERVIEW));
      }
      if (url.includes("/admin/analytics/timeseries")) {
        return Promise.resolve(jsonResponse(EMPTY_SERIES));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("DashboardPage", () => {
  it("renders the heading and all five stat cards from overview data", async () => {
    mockApi();
    renderWithProviders(<DashboardPage />);

    expect(
      screen.getByRole("heading", { name: /dashboard/i }),
    ).toBeInTheDocument();

    expect(screen.getByText(/playing now/i)).toBeInTheDocument();
    expect(screen.getByText(/online in network/i)).toBeInTheDocument();
    expect(screen.getByText(/open worlds/i)).toBeInTheDocument();
    expect(screen.getByText(/active relays/i)).toBeInTheDocument();
    expect(screen.getByText(/total users/i)).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("142")).toBeInTheDocument();
    });
    expect(screen.getByText("7")).toBeInTheDocument();
    expect(screen.getByText("21")).toBeInTheDocument();
    expect(screen.getByText("2")).toBeInTheDocument();
  });

  it("shows the empty chart state when the series has no data", async () => {
    mockApi();
    renderWithProviders(<DashboardPage />);

    await waitFor(() => {
      expect(screen.getByText(/no activity yet/i)).toBeInTheDocument();
    });
  });

  it("exposes the window selector with 24h selected by default", async () => {
    mockApi();
    renderWithProviders(<DashboardPage />);

    const selected = await screen.findByRole("radio", { checked: true });
    expect(selected).toHaveTextContent("24h");
  });
});
