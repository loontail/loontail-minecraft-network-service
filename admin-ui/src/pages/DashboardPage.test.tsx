import { screen, waitFor } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { DashboardPage } from "@/pages/DashboardPage";
import type { AnalyticsOverview } from "@/shared/types";
import { renderWithProviders } from "@/test/renderWithProviders";

const OVERVIEW: AnalyticsOverview = {
  playingNow: 7,
  onlineInNetwork: 21,
  openWorlds: 3,
  activeRelays: 2,
  totalUsers: 142,
};

const originalFetch = globalThis.fetch;

function mockApi() {
  globalThis.fetch = vi.fn((input: RequestInfo | URL) => {
    const url = String(input);
    if (url.includes("/admin/analytics/overview")) {
      return Promise.resolve(
        new Response(JSON.stringify(OVERVIEW), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
    }
    return Promise.reject(new Error(`unexpected fetch: ${url}`));
  }) as typeof fetch;
}

afterEach(() => {
  globalThis.fetch = originalFetch;
  vi.restoreAllMocks();
});

describe("DashboardPage", () => {
  it("renders the heading and the playing-now widget", async () => {
    mockApi();
    renderWithProviders(<DashboardPage />);

    expect(
      screen.getByRole("heading", { name: /dashboard/i }),
    ).toBeInTheDocument();
    expect(screen.getByText(/playing now/i)).toBeInTheDocument();

    await waitFor(() => {
      expect(screen.getByText("7")).toBeInTheDocument();
    });
  });

  it("links to the network page", () => {
    mockApi();
    renderWithProviders(<DashboardPage />);

    const link = screen.getByRole("link", { name: /view network/i });
    expect(link).toHaveAttribute("href", "/network");
  });
});
