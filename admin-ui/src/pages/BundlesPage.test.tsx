import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BundlesPage } from "@/pages/BundlesPage";
import { renderWithProviders } from "@/test/renderWithProviders";
import type { Bundle, DiskSpace } from "@/shared/types";

const SAMPLE_BUILD: Bundle = {
  id: "b1",
  slug: "atm9-overlay",
  name: "ATM9 Overlay",
  description: null,
  version: "1.0.0",
  status: "ready",
  filesCount: 12,
  totalSize: 4096,
  processingError: null,
  lastGeneratedAt: "2026-06-01T00:00:00Z",
  createdAt: "2026-05-01T00:00:00Z",
  updatedAt: "2026-06-01T00:00:00Z",
};

const DISK: DiskSpace = { free: 50_000_000_000, total: 100_000_000_000 };

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

// `useBuilds` returns a bare array; `useDiskSpace` returns the DiskSpace object.
function mockApi({ builds = [] as Bundle[], disk = DISK } = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/admin/bundles/disk-space")) {
        return Promise.resolve(jsonResponse(disk));
      }
      if (url.includes("/admin/bundles/builds")) {
        return Promise.resolve(jsonResponse(builds));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

describe("BundlesPage", () => {
  beforeEach(() => {
    mockApi();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the header, storage card, and empty build state", async () => {
    renderWithProviders(<BundlesPage />);

    expect(
      screen.getByRole("heading", { name: /bundles/i }),
    ).toBeInTheDocument();
    expect(screen.getByText(/storage volume/i)).toBeInTheDocument();

    await waitFor(() =>
      expect(screen.getByText(/no builds yet/i)).toBeInTheDocument(),
    );
  });

  it("renders build rows from the query", async () => {
    mockApi({ builds: [SAMPLE_BUILD] });
    renderWithProviders(<BundlesPage />);

    await waitFor(() =>
      expect(screen.getByText("ATM9 Overlay")).toBeInTheDocument(),
    );
    expect(screen.getByText("atm9-overlay")).toBeInTheDocument();
    expect(screen.getByText("Ready")).toBeInTheDocument();
    expect(
      screen.getByRole("columnheader", { name: /status/i }),
    ).toBeInTheDocument();
  });

  it("shows free space from the disk-space query", async () => {
    renderWithProviders(<BundlesPage />);

    await waitFor(() =>
      expect(screen.getByText(/free/i)).toBeInTheDocument(),
    );
    // Percentage line proves the disk query resolved (50 of 100 GB used).
    expect(screen.getByText(/50% used/i)).toBeInTheDocument();
  });

  it("opens the create-build dialog from the primary action", async () => {
    const user = userEvent.setup();
    renderWithProviders(<BundlesPage />);

    await user.click(screen.getByRole("button", { name: /new build/i }));

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByRole("heading", { name: /create build/i }),
    ).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^name$/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^slug$/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^version$/i)).toBeInTheDocument();
  });
});
