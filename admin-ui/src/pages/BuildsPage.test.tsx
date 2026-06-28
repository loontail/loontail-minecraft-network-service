import { screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BuildsPage } from "@/pages/BuildsPage";
import type { ClientAdmin } from "@/shared/types";
import { renderWithProviders } from "@/test/renderWithProviders";

const SAMPLE_BUILD: ClientAdmin = {
  id: "11111111111111111111111111111111",
  slug: "all-the-mods-9",
  title: "All the Mods 9",
  description: "",
  shortDescription: "",
  available: true,
  minecraftVersion: "1.21.4",
  forgeVersion: null,
  fabricVersion: null,
  runtimeVersion: "21",
  bundleSlug: "all-the-mods-9",
  background: null,
  poster: null,
  titleImage: null,
  screenshots: [],
  keywords: [],
  servers: [],
  bundle: {
    slug: "all-the-mods-9",
    version: "1.0.0",
    status: "ready",
    filesCount: 7,
    manifestUrl: "/api/bundle-registry/builds/all-the-mods-9/manifest",
  },
  published: true,
};

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

function mockApi({ clients = [] as ClientAdmin[] } = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/admin/catalog/clients")) {
        return Promise.resolve(jsonResponse({ clients }));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

describe("BuildsPage", () => {
  beforeEach(() => {
    mockApi();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the header and primary action", () => {
    renderWithProviders(<BuildsPage />);

    expect(
      screen.getByRole("heading", { name: /builds/i }),
    ).toBeInTheDocument();
    expect(
      screen.getAllByRole("button", { name: /new build/i })[0],
    ).toBeInTheDocument();
  });

  it("shows the empty state when there are no builds", async () => {
    renderWithProviders(<BuildsPage />);

    await waitFor(() =>
      expect(screen.getByText(/no builds yet/i)).toBeInTheDocument(),
    );
  });

  it("renders a build row with its title, slug, and file count", async () => {
    mockApi({ clients: [SAMPLE_BUILD] });
    renderWithProviders(<BuildsPage />);

    await waitFor(() =>
      expect(screen.getByText("All the Mods 9")).toBeInTheDocument(),
    );
    expect(screen.getByText("all-the-mods-9")).toBeInTheDocument();
    expect(screen.getByText("1.21.4")).toBeInTheDocument();
    expect(screen.getByText("7")).toBeInTheDocument();
    expect(screen.getByText("Published")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /open/i }),
    ).toBeInTheDocument();
  });

  it("shows an error state (distinct from empty) when the list fails to load", async () => {
    vi.stubGlobal(
      "fetch",
      vi.fn(() =>
        Promise.resolve(
          new Response(JSON.stringify({ message: "boom" }), {
            status: 500,
            headers: { "content-type": "application/json" },
          }),
        ),
      ),
    );
    renderWithProviders(<BuildsPage />);

    await waitFor(() =>
      expect(screen.getByText(/could not load builds/i)).toBeInTheDocument(),
    );
    expect(screen.queryByText(/no builds yet/i)).not.toBeInTheDocument();
  });
});
