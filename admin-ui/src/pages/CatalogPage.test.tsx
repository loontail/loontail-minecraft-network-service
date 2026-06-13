import { screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CatalogPage } from "@/pages/CatalogPage";
import { renderWithProviders } from "@/test/renderWithProviders";
import type {
  Client,
  Keyword,
  ListEnvelope,
  Server,
} from "@/shared/types";

const SAMPLE_CLIENT: Client = {
  id: 1,
  documentId: "11111111-1111-1111-1111-111111111111",
  slug: "all-the-mods-9",
  title: "All the Mods 9",
  description: "",
  shortDescription: "",
  available: true,
  minecraftVersion: "1.21.4",
  forgeVersion: null,
  fabricVersion: null,
  runtimeVersion: "21",
  bundleSlug: "atm9-overlay",
  background: null,
  poster: null,
  titleImage: null,
  screenshots: [],
  keywords: [],
  servers: [],
  createdAt: "2026-01-02T03:04:05Z",
  updatedAt: "2026-01-02T03:04:05Z",
  publishedAt: "2026-01-02T03:04:05Z",
};

const SAMPLE_KEYWORD: Keyword = {
  id: 2,
  documentId: "22222222-2222-2222-2222-222222222222",
  title: "Tech",
  createdAt: "2026-01-02T03:04:05Z",
  updatedAt: "2026-01-02T03:04:05Z",
  publishedAt: null,
};

const SAMPLE_SERVER: Server = {
  id: 3,
  documentId: "33333333-3333-3333-3333-333333333333",
  name: "Survival",
  address: "play.example.net",
  createdAt: "2026-01-02T03:04:05Z",
  updatedAt: "2026-01-02T03:04:05Z",
  publishedAt: "2026-01-02T03:04:05Z",
};

function envelope<T>(rows: T[]): ListEnvelope<T> {
  return {
    data: rows,
    meta: { pagination: { page: 1, pageSize: 25, pageCount: 1, total: rows.length } },
  };
}

function jsonResponse(body: unknown): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    headers: { "content-type": "application/json" },
  });
}

// Route the three catalog read surfaces; default to empty so each test opts in.
function mockApi({
  clients = [] as Client[],
  keywords = [] as Keyword[],
  servers = [] as Server[],
} = {}) {
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL) => {
      const url = String(input);
      if (url.includes("/api/clients")) {
        return Promise.resolve(jsonResponse(envelope(clients)));
      }
      if (url.includes("/api/keywords")) {
        return Promise.resolve(jsonResponse(envelope(keywords)));
      }
      if (url.includes("/api/servers")) {
        return Promise.resolve(jsonResponse(envelope(servers)));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

describe("CatalogPage", () => {
  beforeEach(() => {
    mockApi();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("renders the header and section tabs with clients selected by default", () => {
    renderWithProviders(<CatalogPage />);

    expect(
      screen.getByRole("heading", { name: /catalog/i }),
    ).toBeInTheDocument();

    const tablist = screen.getByRole("tablist");
    expect(
      within(tablist).getByRole("tab", { name: /clients/i }),
    ).toBeInTheDocument();
    expect(
      within(tablist).getByRole("tab", { name: /keywords/i }),
    ).toBeInTheDocument();
    expect(
      within(tablist).getByRole("tab", { name: /servers/i }),
    ).toBeInTheDocument();
  });

  it("shows the empty state for the clients section", async () => {
    renderWithProviders(<CatalogPage />);

    await waitFor(() =>
      expect(screen.getByText(/no clients yet/i)).toBeInTheDocument(),
    );
  });

  it("renders client rows from the query", async () => {
    mockApi({ clients: [SAMPLE_CLIENT] });
    renderWithProviders(<CatalogPage />);

    await waitFor(() =>
      expect(screen.getByText("All the Mods 9")).toBeInTheDocument(),
    );
    expect(screen.getByText("all-the-mods-9")).toBeInTheDocument();
    expect(screen.getByText("1.21.4")).toBeInTheDocument();
    expect(screen.getByText("Published")).toBeInTheDocument();
  });

  it("opens the new-client dialog from the primary action", async () => {
    const user = userEvent.setup();
    renderWithProviders(<CatalogPage />);

    // The empty state renders before the create button is meaningfully testable.
    await waitFor(() =>
      expect(screen.getByText(/no clients yet/i)).toBeInTheDocument(),
    );

    await user.click(
      screen.getAllByRole("button", { name: /new client/i })[0],
    );

    const dialog = await screen.findByRole("dialog");
    expect(
      within(dialog).getByRole("heading", { name: /new client/i }),
    ).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^slug$/i)).toBeInTheDocument();
    expect(within(dialog).getByLabelText(/^title$/i)).toBeInTheDocument();
    expect(
      within(dialog).getByLabelText(/minecraft version/i),
    ).toBeInTheDocument();
  });

  it("switches to the keywords section and renders its rows", async () => {
    const user = userEvent.setup();
    mockApi({ keywords: [SAMPLE_KEYWORD] });
    renderWithProviders(<CatalogPage />);

    await user.click(screen.getByRole("tab", { name: /keywords/i }));

    await waitFor(() => expect(screen.getByText("Tech")).toBeInTheDocument());
    expect(screen.getByText("Draft")).toBeInTheDocument();
  });

  it("switches to the servers section and renders its rows", async () => {
    const user = userEvent.setup();
    mockApi({ servers: [SAMPLE_SERVER] });
    renderWithProviders(<CatalogPage />);

    await user.click(screen.getByRole("tab", { name: /servers/i }));

    await waitFor(() =>
      expect(screen.getByText("Survival")).toBeInTheDocument(),
    );
    expect(screen.getByText("play.example.net")).toBeInTheDocument();
  });
});
