import { screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { BuildFilesTab } from "@/features/builds/BuildFilesTab";
import * as downloadModule from "@/features/builds/download";
import type {
  BuildAdmin,
  BundleArtifact,
  BundleWithArtifacts,
} from "@/shared/types";
import {
  renderWithProviders,
  setupUser,
} from "@/test/renderWithProviders";

function artifact(
  relativePath: string,
  isDir: boolean,
  extra: Partial<BundleArtifact> = {},
): BundleArtifact {
  const name = relativePath.split("/").pop() ?? relativePath;
  return {
    id: `id-${relativePath}`,
    bundleId: "bundle-1",
    relativePath,
    name,
    category: isDir ? "dir" : "file",
    size: isDir ? 0 : 2048,
    sha256: isDir ? null : "abcdef0123456789".repeat(4),
    isDir,
    downloadOnce: false,
    fileModifiedAt: null,
    ...extra,
  };
}

const BUILD: BundleWithArtifacts = {
  id: "bundle-1",
  slug: "atm9-overlay",
  name: "ATM9 Overlay",
  description: null,
  version: "1.0.0",
  status: "ready",
  filesCount: 2,
  totalSize: 4096,
  processingError: null,
  lastGeneratedAt: "2026-06-01T00:00:00Z",
  createdAt: "2026-05-01T00:00:00Z",
  updatedAt: "2026-06-01T00:00:00Z",
  artifacts: [
    artifact("mods", true),
    artifact("mods/a.jar", false),
    artifact("config", true),
    artifact("config/x.toml", false),
  ],
};

// A BuildAdmin with a verified owned bundle (drives `useBuildFiles(bundle.slug)`).
function buildAdmin(overrides: Partial<BuildAdmin> = {}): BuildAdmin {
  return {
    id: "build-1",
    slug: "atm9-overlay",
    title: "ATM9 Overlay",
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
    bundle: {
      slug: "atm9-overlay",
      version: "1.0.0",
      status: "ready",
      filesCount: 2,
      manifestUrl: "/api/bundle-registry/builds/atm9-overlay/manifest",
    },
    published: true,
    ...overrides,
  };
}

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

const fetchCalls: { url: string; init?: RequestInit }[] = [];

// The build returned by the GET endpoint, mutable so a test can simulate the folder
// disappearing after a delete (the query refetches on invalidation).
let currentBuild: BundleWithArtifacts = BUILD;
let getStatus = 200;
let getErrorMessage = "bundle missing";

function mockApi(build: BundleWithArtifacts = BUILD) {
  fetchCalls.length = 0;
  currentBuild = build;
  getStatus = 200;
  getErrorMessage = "bundle missing";
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      const method = init?.method ?? "GET";
      fetchCalls.push({ url, init });
      if (url.includes(`/admin/bundles/builds/${build.slug}/files/bulk-delete`)) {
        return Promise.resolve(jsonResponse({ deleted: 1 }));
      }
      if (url.includes(`/admin/bundles/builds/${build.slug}/files/move`)) {
        return Promise.resolve(jsonResponse(currentBuild));
      }
      if (url.includes(`/admin/bundles/builds/${build.slug}/files`)) {
        // file upload (POST) or download-once (PUT) return the bundle.
        return Promise.resolve(jsonResponse(currentBuild));
      }
      if (
        method === "DELETE" &&
        url.includes(`/admin/bundles/builds/${build.slug}/files/`)
      ) {
        return Promise.resolve(jsonResponse({ message: "ok", slug: build.slug }));
      }
      if (url.includes(`/admin/bundles/builds/${build.slug}`)) {
        if (getStatus !== 200) {
          return Promise.resolve(
            jsonResponse(
              { error: { code: "not_found", message: getErrorMessage } },
              getStatus,
            ),
          );
        }
        return Promise.resolve(jsonResponse(currentBuild));
      }
      return Promise.reject(new Error(`unexpected fetch: ${url}`));
    }),
  );
}

// The file collection renders as a native ARIA table (shadcn Table, list view is
// the default).
function table(): HTMLElement {
  return screen.getByRole("table", { name: /build files/i });
}

// Await the table once the build query resolves out of the loading skeleton.
function findTable(): Promise<HTMLElement> {
  return screen.findByRole("table", { name: /build files/i });
}

describe("BuildFilesTab", () => {
  beforeEach(() => {
    mockApi();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("shows the heal CTA when the build has no bundle", () => {
    renderWithProviders(<BuildFilesTab build={buildAdmin({ bundle: null })} />);
    expect(
      screen.getByRole("button", { name: /set up file storage/i }),
    ).toBeInTheDocument();
  });

  it("shows a retryable error when the bundle no longer exists (404)", async () => {
    getStatus = 404;
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);
    expect(
      await screen.findByText(/bundle that no longer exists/i),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /retry/i })).toBeInTheDocument();
  });

  // The status, not the wording, decides which copy renders: a server fault whose
  // message happens to say "not found" must not claim the bundle is gone.
  it("keeps the generic copy for a 500 whose message contains “not found”", async () => {
    getStatus = 500;
    getErrorMessage = "artifact not found during manifest regen";
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);
    expect(
      await screen.findByText(/could not be loaded/i),
    ).toBeInTheDocument();
    expect(
      screen.queryByText(/bundle that no longer exists/i),
    ).not.toBeInTheDocument();
  });

  it("renders the file table with folders and the New menu", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    const rows = within(await findTable())
      .getAllByRole("row")
      .map((row) => row.textContent);
    expect(rows.some((t) => t?.includes("mods"))).toBe(true);
    expect(rows.some((t) => t?.includes("config"))).toBe(true);

    await user.click(screen.getByRole("button", { name: /^new$/i }));
    expect(
      await screen.findByRole("menuitem", { name: /new folder/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: /upload file/i }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("menuitem", { name: /upload zip/i }),
    ).toBeInTheDocument();
  });

  it("renders the footer status row", async () => {
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);
    await waitFor(() => expect(screen.getByText("Ready")).toBeInTheDocument());
    expect(screen.getByText(/generated/i)).toBeInTheDocument();
    expect(screen.getByText("4.0 KB")).toBeInTheDocument();
  });

  it("toggles between list and grid views", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);
    await findTable();

    await user.click(screen.getByRole("button", { name: /grid view/i }));
    // Grid view drops the table for native cards; the "Actions for mods" kebab
    // still exists but no role="table".
    await waitFor(() =>
      expect(
        screen.queryByRole("table", { name: /build files/i }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("button", { name: /actions for mods/i }),
    ).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /list view/i }));
    expect(table()).toBeInTheDocument();
  });

  it("navigates into a folder on a single click of its name", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    // Single click on the folder NAME button navigates in (no double-click).
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    expect(within(table()).getByText("a.jar")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^root$/i })).toBeInTheDocument();
  });

  it("selects a file on a single click of its name (no download)", async () => {
    const user = setupUser();
    const downloadSpy = vi
      .spyOn(downloadModule, "downloadFile")
      .mockImplementation(() => undefined);
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    // Clicking the file NAME selects it (the SelectionToolbar appears), and does
    // NOT trigger a download.
    await user.click(within(table()).getByRole("button", { name: /^a\.jar$/i }));
    expect(await screen.findByText(/1 selected/i)).toBeInTheDocument();
    expect(downloadSpy).not.toHaveBeenCalled();
  });

  it("selects an entry via its row checkbox", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    await user.click(
      within(table()).getByRole("checkbox", { name: /select a\.jar/i }),
    );
    expect(await screen.findByText(/1 selected/i)).toBeInTheDocument();
  });

  it("downloads a file from the context menu", async () => {
    const user = setupUser();
    const downloadSpy = vi
      .spyOn(downloadModule, "downloadFile")
      .mockImplementation(() => undefined);
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    await user.click(
      within(table()).getByRole("button", { name: /actions for a\.jar/i }),
    );
    await user.click(
      await screen.findByRole("menuitem", { name: /^download$/i }),
    );

    expect(downloadSpy).toHaveBeenCalledWith("atm9-overlay", "mods/a.jar");
  });

  it("staggers a multi-file download instead of firing every anchor at once", async () => {
    // A flat build so several files live in the same folder and can be selected together.
    const flat: BundleWithArtifacts = {
      ...BUILD,
      artifacts: [
        artifact("a.jar", false),
        artifact("b.jar", false),
        artifact("c.jar", false),
      ],
    };
    mockApi(flat);
    const user = setupUser();
    const downloadSpy = vi
      .spyOn(downloadModule, "downloadFile")
      .mockImplementation(() => undefined);
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(within(table()).getByRole("checkbox", { name: /select all/i }));
    expect(await screen.findByText(/3 selected/i)).toBeInTheDocument();

    await user.click(screen.getByRole("button", { name: /^download$/i }));

    // Browsers throttle a burst of synthetic clicks, so only the first fires now.
    expect(downloadSpy).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(downloadSpy).toHaveBeenCalledTimes(3), {
      timeout: 3000,
    });
  });

  it("bulk-deletes the entries selected in the table", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    await user.click(
      within(table()).getByRole("checkbox", { name: /select a\.jar/i }),
    );
    expect(await screen.findByText(/1 selected/i)).toBeInTheDocument();

    const deleteButton = await screen.findByRole("button", { name: /^delete$/i });
    await user.click(deleteButton);

    const dialog = await screen.findByRole("dialog");
    await user.click(
      within(dialog).getByRole("button", { name: /^delete selected$/i }),
    );

    await waitFor(() => {
      expect(
        fetchCalls.some((c) => c.url.includes("/files/bulk-delete")),
      ).toBe(true);
    });
  });

  it("moves a selected entry via the Move dialog", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    await user.click(
      within(table()).getByRole("checkbox", { name: /select a\.jar/i }),
    );
    await user.click(await screen.findByRole("button", { name: /^move$/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^config$/i }));
    await user.click(within(dialog).getByRole("button", { name: /move here/i }));

    await waitFor(() => {
      expect(fetchCalls.some((c) => c.url.includes("/files/move"))).toBe(true);
    });
  });

  it("selects all artifact-backed children via the header checkbox", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));

    // Header "Select all" picks only artifact-backed children — there is one file
    // in "mods", so the counter reads "1 selected" (never lies).
    await user.click(within(table()).getByRole("checkbox", { name: /select all/i }));
    expect(await screen.findByText(/1 selected/i)).toBeInTheDocument();
  });

  it("uploads OS files dropped via the hidden upload input", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);
    await findTable();

    await user.click(screen.getByRole("button", { name: /^new$/i }));
    await user.click(await screen.findByRole("menuitem", { name: /upload file/i }));

    const file = new File(["jar bytes"], "extra.jar", {
      type: "application/java-archive",
    });
    const input = document.querySelector<HTMLInputElement>(
      'input[type="file"]:not([accept])',
    );
    expect(input).not.toBeNull();
    await user.upload(input!, file);

    await waitFor(() => {
      expect(
        fetchCalls.some(
          (c) =>
            c.url.endsWith("/admin/bundles/builds/atm9-overlay/files") &&
            (c.init?.method ?? "GET") === "POST",
        ),
      ).toBe(true);
    });
  });

  it("keyboard: Enter on a folder name navigates in", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    const modsName = screen.getByRole("button", { name: /^mods$/i });
    modsName.focus();
    await user.keyboard("{Enter}");

    expect(within(table()).getByText("a.jar")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /^root$/i })).toBeInTheDocument();
  });

  it("clamps the current folder back to root when it disappears", async () => {
    const user = setupUser();
    renderWithProviders(<BuildFilesTab build={buildAdmin()} />);

    await findTable();
    await user.click(screen.getByRole("button", { name: /^config$/i }));
    expect(screen.getByRole("button", { name: /^root$/i })).toBeInTheDocument();

    // Delete the only file in "config"; the refetched build no longer has it.
    currentBuild = {
      ...BUILD,
      artifacts: [artifact("mods", true), artifact("mods/a.jar", false)],
    };
    await user.click(
      within(table()).getByRole("button", { name: /actions for x\.toml/i }),
    );
    await user.click(await screen.findByRole("menuitem", { name: /^delete$/i }));
    const dialog = await screen.findByRole("dialog");
    await user.click(within(dialog).getByRole("button", { name: /^delete$/i }));

    // currentPath self-heals to root: the "config" breadcrumb is gone and the
    // root-level "mods" folder is shown again.
    await waitFor(() => {
      expect(
        screen.queryByRole("button", { name: /^root$/i }),
      ).not.toBeInTheDocument();
    });
    expect(within(table()).getByText("mods")).toBeInTheDocument();
  });
});
