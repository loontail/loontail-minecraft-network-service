import { screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import type { TreeEntry } from "@/features/builds/fileTree";
import { NewFolderDialog } from "@/features/builds/NewFolderDialog";
import { RenameDialog } from "@/features/builds/RenameDialog";
import {
  renderWithProviders,
  setupUser,
} from "@/test/renderWithProviders";

const ENTRY: TreeEntry = {
  relativePath: "mods/a.jar",
  name: "a.jar",
  isDir: false,
  artifact: {
    id: "artifact-1",
    bundleId: "bundle-1",
    relativePath: "mods/a.jar",
    name: "a.jar",
    category: "file",
    size: 2048,
    sha256: "a".repeat(64),
    isDir: false,
    downloadOnce: false,
    fileModifiedAt: null,
  },
};

const fetchCalls: { url: string; body: unknown }[] = [];

function mockApi() {
  fetchCalls.length = 0;
  vi.stubGlobal(
    "fetch",
    vi.fn((input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input);
      let body: unknown;
      if (typeof init?.body === "string") {
        body = JSON.parse(init.body);
      }
      fetchCalls.push({ url, body });
      return Promise.resolve(
        new Response(JSON.stringify({ slug: "atm9", artifacts: [] }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
    }),
  );
}

describe("RenameDialog", () => {
  beforeEach(mockApi);
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("renames within the current folder", async () => {
    const user = setupUser();
    renderWithProviders(
      <RenameDialog
        slug="atm9"
        entry={ENTRY}
        open
        onOpenChange={() => undefined}
      />,
    );

    const input = screen.getByLabelText(/new name/i);
    await user.clear(input);
    await user.type(input, "b.jar");
    await user.click(screen.getByRole("button", { name: /^rename$/i }));

    await waitFor(() => expect(fetchCalls).toHaveLength(1));
    expect(fetchCalls[0].url).toContain(
      "/admin/bundles/builds/atm9/files/artifact-1/rename",
    );
    expect(fetchCalls[0].body).toEqual({ newRelativePath: "mods/b.jar" });
  });

  // A name with a separator is a MOVE the backend accepts silently, so the dialog
  // must refuse it rather than report "Renamed" and drop the file elsewhere.
  it("refuses a name containing a path separator", async () => {
    const user = setupUser();
    renderWithProviders(
      <RenameDialog
        slug="atm9"
        entry={ENTRY}
        open
        onOpenChange={() => undefined}
      />,
    );

    const input = screen.getByLabelText(/new name/i);
    await user.clear(input);
    await user.type(input, "sub/b.jar");

    expect(await screen.findByText(/can’t contain/i)).toBeInTheDocument();
    const submit = screen.getByRole("button", { name: /^rename$/i });
    expect(submit).toBeDisabled();
    await user.click(submit);
    expect(fetchCalls).toHaveLength(0);
  });
});

describe("NewFolderDialog", () => {
  beforeEach(mockApi);
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("creates a folder under the current path", async () => {
    const user = setupUser();
    renderWithProviders(
      <NewFolderDialog
        slug="atm9"
        currentPath="mods"
        open
        onOpenChange={() => undefined}
      />,
    );

    await user.type(screen.getByLabelText(/folder name/i), "sub");
    await user.click(screen.getByRole("button", { name: /create folder/i }));

    await waitFor(() => expect(fetchCalls).toHaveLength(1));
    expect(fetchCalls[0].body).toEqual({ relativePath: "mods/sub" });
  });

  it("refuses a nested folder name", async () => {
    const user = setupUser();
    renderWithProviders(
      <NewFolderDialog
        slug="atm9"
        currentPath="mods"
        open
        onOpenChange={() => undefined}
      />,
    );

    await user.type(screen.getByLabelText(/folder name/i), "a/b");

    expect(await screen.findByText(/can’t contain/i)).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: /create folder/i }),
    ).toBeDisabled();
    expect(fetchCalls).toHaveLength(0);
  });
});
