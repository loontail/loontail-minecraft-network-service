import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import { FileGrid } from "@/features/builds/FileGrid";
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
  overrides: Partial<React.ComponentProps<typeof FileGrid>> = {},
) {
  const onToggle = vi.fn();
  const onToggleAll = vi.fn();
  const onAction = vi.fn();
  const onOpenMenu = vi.fn();
  render(
    <FileGrid
      entries={ENTRIES}
      selectedKeys={new Set()}
      onToggle={onToggle}
      onToggleAll={onToggleAll}
      allSelected={false}
      someSelected={false}
      onAction={onAction}
      onOpenMenu={onOpenMenu}
      {...overrides}
    />,
  );
  return { onToggle, onToggleAll, onAction, onOpenMenu };
}

describe("FileGrid", () => {
  it("calls onAction when a folder card body is clicked", async () => {
    const user = userEvent.setup();
    const { onAction, onToggle } = setup();
    await user.click(screen.getByRole("button", { name: /^mods$/i }));
    expect(onAction).toHaveBeenCalledWith("mods");
    expect(onToggle).not.toHaveBeenCalled();
  });

  it("calls onAction when a file card body is clicked", async () => {
    const user = userEvent.setup();
    const { onAction } = setup();
    await user.click(screen.getByRole("button", { name: /^a\.jar$/i }));
    expect(onAction).toHaveBeenCalledWith("a.jar");
  });

  it("toggles via the corner checkbox without opening", async () => {
    const user = userEvent.setup();
    const { onToggle, onAction } = setup();
    await user.click(screen.getByRole("checkbox", { name: /select a\.jar/i }));
    expect(onToggle).toHaveBeenCalledWith("a.jar", false);
    expect(onAction).not.toHaveBeenCalled();
  });

  it("exposes the card selection state to assistive tech", () => {
    setup({ selectedKeys: new Set(["a.jar"]) });
    expect(
      screen.getByRole("checkbox", { name: /select a\.jar/i }),
    ).toHaveAttribute("aria-checked", "true");
    expect(
      screen.getByRole("checkbox", { name: /select b\.jar/i }),
    ).toHaveAttribute("aria-checked", "false");
  });

  it("passes shiftKey through the corner checkbox for range selection", async () => {
    const user = userEvent.setup();
    const { onToggle } = setup();
    await user.keyboard("{Shift>}");
    await user.click(screen.getByRole("checkbox", { name: /select c\.jar/i }));
    await user.keyboard("{/Shift}");
    expect(onToggle).toHaveBeenCalledWith("c.jar", true);
  });

  it("range-selects from the keyboard too (Shift+Enter on the corner checkbox)", async () => {
    const user = userEvent.setup();
    const { onToggle } = setup();
    screen.getByRole("checkbox", { name: /select c\.jar/i }).focus();
    await user.keyboard("{Shift>}{Enter}{/Shift}");
    expect(onToggle).toHaveBeenCalledWith("c.jar", true);
  });

  it("offers select-all like the list view, with a mixed state", async () => {
    const user = userEvent.setup();
    const { onToggleAll } = setup({ someSelected: true });
    const selectAll = screen.getByRole("checkbox", { name: /select all/i });
    expect(selectAll).toHaveAttribute("aria-checked", "mixed");
    await user.click(selectAll);
    expect(onToggleAll).toHaveBeenCalledWith(true);
  });

  it("hides select-all in a folder-only listing, where it could never check", () => {
    setup({ entries: [folderEntry("mods"), folderEntry("config")] });
    expect(
      screen.queryByRole("checkbox", { name: /select all/i }),
    ).not.toBeInTheDocument();
  });

  it("folders have no corner checkbox", () => {
    setup();
    expect(
      screen.queryByRole("checkbox", { name: /select mods/i }),
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

  it("reflects selection state on the card", () => {
    setup({ selectedKeys: new Set(["a.jar"]) });
    const card = screen
      .getByRole("button", { name: /^a\.jar$/i })
      .closest("[data-state]");
    expect(card).toHaveAttribute("data-state", "selected");
  });
});
