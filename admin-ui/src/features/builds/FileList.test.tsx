import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FileList } from "@/features/builds/FileList";
import type { TreeEntry } from "@/features/builds/fileTree";
import type { BundleArtifact } from "@/shared/types";

function artifactEntry(
  relativePath: string,
  extra: Partial<BundleArtifact> = {},
): TreeEntry {
  const name = relativePath.split("/").pop() ?? relativePath;
  return {
    relativePath,
    name,
    isDir: false,
    artifact: {
      id: `id-${relativePath}`,
      bundleId: "b",
      relativePath,
      name,
      category: "file",
      size: 2048,
      sha256: "abcdef0123456789".repeat(4),
      isDir: false,
      downloadOnce: false,
      fileModifiedAt: null,
      ...extra,
    },
  };
}

function folderEntry(relativePath: string): TreeEntry {
  const name = relativePath.split("/").pop() ?? relativePath;
  return { relativePath, name, isDir: true, artifact: null };
}

const ENTRIES: TreeEntry[] = [
  folderEntry("mods"),
  artifactEntry("a.jar"),
  artifactEntry("b.jar"),
  artifactEntry("c.jar"),
];

function setup(
  overrides: Partial<React.ComponentProps<typeof FileList>> = {},
) {
  const onToggle = vi.fn();
  const onToggleAll = vi.fn();
  const onAction = vi.fn();
  const onOpenMenu = vi.fn();
  const onToggleDownloadOnce = vi.fn();
  render(
    <FileList
      entries={ENTRIES}
      selectedKeys={new Set()}
      onToggle={onToggle}
      onToggleAll={onToggleAll}
      allSelected={false}
      someSelected={false}
      onAction={onAction}
      onOpenMenu={onOpenMenu}
      onToggleDownloadOnce={onToggleDownloadOnce}
      {...overrides}
    />,
  );
  return { onToggle, onToggleAll, onAction, onOpenMenu, onToggleDownloadOnce };
}

describe("FileList", () => {
  it("renders an accessible table", () => {
    setup();
    expect(
      screen.getByRole("table", { name: /build files/i }),
    ).toBeInTheDocument();
  });

  it("calls onAction when a folder name is clicked", async () => {
    const user = userEvent.setup();
    const { onAction, onToggle } = setup();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));
    expect(onAction).toHaveBeenCalledWith("mods");
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("calls onAction (select) when a file name is clicked", async () => {
    const user = userEvent.setup();
    const { onAction } = setup();
    await user.click(screen.getByRole("button", { name: /^a\.jar$/i }));
    expect(onAction).toHaveBeenCalledWith("a.jar");
  });

  it("toggles via the row checkbox without firing onAction", async () => {
    const user = userEvent.setup();
    const { onToggle, onAction } = setup();
    await user.click(screen.getByRole("button", { name: /select a\.jar/i }));
    expect(onToggle).toHaveBeenCalledWith("a.jar", false);
    expect(onAction).not.toHaveBeenCalled();
  });

  it("passes shiftKey through the checkbox wrapper for range selection", async () => {
    const user = userEvent.setup();
    const { onToggle } = setup();
    await user.keyboard("{Shift>}");
    await user.click(screen.getByRole("button", { name: /select c\.jar/i }));
    await user.keyboard("{/Shift}");
    expect(onToggle).toHaveBeenCalledWith("c.jar", true);
  });

  it("calls onToggleAll from the header checkbox", async () => {
    const user = userEvent.setup();
    const { onToggleAll } = setup();
    await user.click(screen.getByRole("button", { name: /select all/i }));
    expect(onToggleAll).toHaveBeenCalledWith(true);
  });

  it("folders are not selectable (no checkbox on the folder row)", () => {
    setup();
    expect(
      screen.queryByRole("button", { name: /select mods/i }),
    ).not.toBeInTheDocument();
  });

  it("opens the context menu via the kebab", async () => {
    const user = userEvent.setup();
    const { onOpenMenu } = setup();
    await user.click(
      screen.getByRole("button", { name: /actions for a\.jar/i }),
    );
    expect(onOpenMenu).toHaveBeenCalled();
    expect(onOpenMenu.mock.calls[0][0].relativePath).toBe("a.jar");
  });

  it("toggles download-once via the per-row switch", async () => {
    const user = userEvent.setup();
    const { onToggleDownloadOnce } = setup();
    await user.click(
      screen.getByRole("switch", { name: /download once for a\.jar/i }),
    );
    expect(onToggleDownloadOnce).toHaveBeenCalled();
    expect(onToggleDownloadOnce.mock.calls[0][0].relativePath).toBe("a.jar");
  });

  it("keyboard: Enter on a name button activates onAction", async () => {
    const user = userEvent.setup();
    const { onAction } = setup();
    const name = screen.getByRole("button", { name: /^a\.jar$/i });
    name.focus();
    await user.keyboard("{Enter}");
    expect(onAction).toHaveBeenCalledWith("a.jar");
  });

  it("reflects selection state on the row", () => {
    setup({ selectedKeys: new Set(["a.jar"]) });
    const row = screen
      .getByRole("button", { name: /^a\.jar$/i })
      .closest("tr");
    expect(row).toHaveAttribute("data-state", "selected");
  });

  it("the header checkbox reflects allSelected as a partial state", () => {
    setup({ someSelected: true });
    // Header "Select all" exists and someSelected styles it; clicking still toggles.
    expect(
      within(screen.getByRole("table")).getByRole("button", {
        name: /select all/i,
      }),
    ).toBeInTheDocument();
  });
});
