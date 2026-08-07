import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { describe, expect, it, vi } from "vitest";

import type { FileTreeNode } from "@/features/builds/fileTree";
import { MoveDialog } from "@/features/builds/MoveDialog";

function dir(relativePath: string, children: FileTreeNode[] = []): FileTreeNode {
  const name = relativePath.split("/").pop() ?? relativePath;
  return {
    id: relativePath,
    relativePath,
    name,
    isDir: true,
    artifact: null,
    children,
  };
}

function file(relativePath: string): FileTreeNode {
  const name = relativePath.split("/").pop() ?? relativePath;
  return {
    id: relativePath,
    relativePath,
    name,
    isDir: false,
    artifact: null,
    children: [],
  };
}

// mods/ with a nested sub/, plus a sibling config/ and a root-level file.
const ROOTS: FileTreeNode[] = [
  dir("mods", [dir("mods/sub", [file("mods/sub/deep.jar")]), file("mods/a.jar")]),
  dir("config", [file("config/x.toml")]),
  file("readme.txt"),
];

function setup(sourcePaths: string[], onMove = vi.fn()) {
  render(
    <MoveDialog
      open
      onOpenChange={() => undefined}
      roots={ROOTS}
      sourcePaths={sourcePaths}
      pending={false}
      onMove={onMove}
    />,
  );
  return { onMove };
}

function destination(name: string): HTMLElement {
  return screen.getByRole("button", { name: new RegExp(`^${name}$`, "i") });
}

describe("MoveDialog destinations", () => {
  it("lists Root plus every folder, and no files", () => {
    setup(["readme.txt"]);
    expect(destination("Root")).toBeInTheDocument();
    expect(destination("mods")).toBeInTheDocument();
    expect(destination("sub")).toBeInTheDocument();
    expect(destination("config")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^a\.jar$/i }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /^readme\.txt$/i }),
    ).not.toBeInTheDocument();
  });

  // The guard that stops an admin moving a folder into itself: the source and every
  // folder beneath it must be unreachable as a destination.
  it("disables the source folder and its whole subtree", () => {
    setup(["mods"]);
    expect(destination("mods")).toBeDisabled();
    expect(destination("sub")).toBeDisabled();
    expect(destination("config")).toBeEnabled();
    expect(destination("Root")).toBeEnabled();
  });

  it("disables only the subtree of the source, not a name-prefixed sibling", () => {
    const roots: FileTreeNode[] = [dir("mods"), dir("mods-extra")];
    render(
      <MoveDialog
        open
        onOpenChange={() => undefined}
        roots={roots}
        sourcePaths={["mods"]}
        pending={false}
        onMove={vi.fn()}
      />,
    );
    expect(destination("mods")).toBeDisabled();
    // "mods-extra" merely starts with "mods"; it is not a descendant.
    expect(destination("mods-extra")).toBeEnabled();
  });

  it("disables the union of subtrees for a multi-source move", () => {
    setup(["mods/sub", "config"]);
    expect(destination("sub")).toBeDisabled();
    expect(destination("config")).toBeDisabled();
    expect(destination("mods")).toBeEnabled();
    expect(destination("Root")).toBeEnabled();
  });

  it("keeps Move here disabled until a legal destination is picked", async () => {
    const user = userEvent.setup({ delay: null });
    const { onMove } = setup(["mods"]);

    const confirm = screen.getByRole("button", { name: /move here/i });
    expect(confirm).toBeDisabled();

    await user.click(destination("config"));
    expect(confirm).toBeEnabled();
    await user.click(confirm);
    expect(onMove).toHaveBeenCalledWith("config");
  });

  it("moves to the build root as the empty target dir", async () => {
    const user = userEvent.setup({ delay: null });
    const { onMove } = setup(["mods/a.jar"]);

    await user.click(destination("Root"));
    await user.click(screen.getByRole("button", { name: /move here/i }));
    expect(onMove).toHaveBeenCalledWith("");
  });
});
