import { screen, waitFor } from "@testing-library/react";
import { describe, expect, it } from "vitest";

import { DashboardPage } from "@/pages/DashboardPage";
import { renderWithProviders } from "@/test/renderWithProviders";

describe("DashboardPage", () => {
  it("renders the dashboard heading and overview stat cards", async () => {
    renderWithProviders(<DashboardPage />);

    expect(
      screen.getByRole("heading", { name: /dashboard/i }),
    ).toBeInTheDocument();

    // Stat labels render immediately from the stub-backed query.
    await waitFor(() => {
      expect(screen.getByText(/playing now/i)).toBeInTheDocument();
    });
    // "Online in network" appears both as a stat label and the chart title.
    expect(screen.getAllByText(/online in network/i).length).toBeGreaterThan(0);
    expect(screen.getByText(/open worlds/i)).toBeInTheDocument();
    expect(screen.getByText(/total users/i)).toBeInTheDocument();
  });
});
