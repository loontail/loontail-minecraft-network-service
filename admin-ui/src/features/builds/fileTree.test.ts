import { describe, expect, it } from "vitest";

import {
  breadcrumbs,
  buildTree,
  childrenOf,
  type FileTreeNode,
  joinPath,
  parentPath,
  segmentError,
} from "@/features/builds/fileTree";
import type { BundleArtifact } from "@/shared/types";

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
    size: isDir ? 0 : 100,
    sha256: isDir ? null : "a".repeat(64),
    isDir,
    downloadOnce: false,
    fileModifiedAt: null,
    ...extra,
  };
}

const FLAT: BundleArtifact[] = [
  artifact("mods", true),
  artifact("mods/a.jar", false),
  artifact("config", true),
  artifact("config/x.toml", false),
];

describe("childrenOf", () => {
  it("returns top-level folders at the root", () => {
    const names = childrenOf(FLAT, "").map((entry) => entry.name);
    expect(names).toEqual(["config", "mods"]);
  });

  it("returns the immediate children of a folder", () => {
    const children = childrenOf(FLAT, "mods");
    expect(children).toHaveLength(1);
    expect(children[0].relativePath).toBe("mods/a.jar");
    expect(children[0].isDir).toBe(false);
    expect(children[0].artifact?.id).toBe("id-mods/a.jar");
  });

  it("does not leak grandchildren into a folder's children", () => {
    const nested: BundleArtifact[] = [
      artifact("mods", true),
      artifact("mods/sub", true),
      artifact("mods/sub/deep.jar", false),
    ];
    const rootNames = childrenOf(nested, "").map((e) => e.name);
    expect(rootNames).toEqual(["mods"]);
    const modsNames = childrenOf(nested, "mods").map((e) => e.name);
    expect(modsNames).toEqual(["sub"]);
    const subNames = childrenOf(nested, "mods/sub").map((e) => e.name);
    expect(subNames).toEqual(["deep.jar"]);
  });

  it("sorts folders before files, alphabetically within each group", () => {
    const mixed: BundleArtifact[] = [
      artifact("zeta.txt", false),
      artifact("alpha.txt", false),
      artifact("beta", true),
      artifact("alpha-dir", true),
    ];
    const names = childrenOf(mixed, "").map((e) => e.name);
    expect(names).toEqual(["alpha-dir", "beta", "alpha.txt", "zeta.txt"]);
  });

  it("synthesizes implied folders for files with no explicit dir row", () => {
    const implied: BundleArtifact[] = [
      artifact("data/inner/file.json", false),
    ];
    const root = childrenOf(implied, "");
    expect(root).toHaveLength(1);
    expect(root[0].name).toBe("data");
    expect(root[0].isDir).toBe(true);
    expect(root[0].artifact).toBeNull();

    const data = childrenOf(implied, "data");
    expect(data).toHaveLength(1);
    expect(data[0].name).toBe("inner");
    expect(data[0].isDir).toBe(true);

    const inner = childrenOf(implied, "data/inner");
    expect(inner.map((e) => e.name)).toEqual(["file.json"]);
  });

  it("prefers the explicit dir row over an implied one", () => {
    const both: BundleArtifact[] = [
      artifact("mods", true),
      artifact("mods/a.jar", false),
    ];
    const [folder] = childrenOf(both, "");
    expect(folder.name).toBe("mods");
    expect(folder.artifact?.id).toBe("id-mods");
  });
});

describe("buildTree", () => {
  function names(nodes: FileTreeNode[]): string[] {
    return nodes.map((node) => node.name);
  }

  it("nests children under their folder with relativePath ids", () => {
    const roots = buildTree(FLAT);
    expect(names(roots)).toEqual(["config", "mods"]);

    const mods = roots.find((n) => n.name === "mods");
    expect(mods).toBeDefined();
    expect(mods?.id).toBe("mods");
    expect(mods?.isDir).toBe(true);
    expect(mods?.artifact?.id).toBe("id-mods");
    expect(names(mods?.children ?? [])).toEqual(["a.jar"]);

    const aJar = mods?.children[0];
    expect(aJar?.id).toBe("mods/a.jar");
    expect(aJar?.isDir).toBe(false);
    expect(aJar?.artifact?.id).toBe("id-mods/a.jar");
    expect(aJar?.children).toEqual([]);
  });

  it("synthesizes implied folders (no artifact) for deep files", () => {
    const roots = buildTree([artifact("data/inner/file.json", false)]);
    expect(names(roots)).toEqual(["data"]);
    const data = roots[0];
    expect(data.isDir).toBe(true);
    expect(data.artifact).toBeNull();

    const inner = data.children[0];
    expect(inner.name).toBe("inner");
    expect(inner.id).toBe("data/inner");
    expect(inner.isDir).toBe(true);
    expect(inner.artifact).toBeNull();

    const file = inner.children[0];
    expect(file.name).toBe("file.json");
    expect(file.id).toBe("data/inner/file.json");
    expect(file.artifact?.id).toBe("id-data/inner/file.json");
  });

  it("prefers the explicit dir row over the implied folder", () => {
    const roots = buildTree([
      artifact("mods/a.jar", false),
      artifact("mods", true),
    ]);
    expect(roots).toHaveLength(1);
    expect(roots[0].artifact?.id).toBe("id-mods");
  });

  it("sorts every level folders-first then alphabetically", () => {
    const mixed: BundleArtifact[] = [
      artifact("zeta.txt", false),
      artifact("alpha.txt", false),
      artifact("beta", true),
      artifact("alpha-dir", true),
      artifact("beta/z.txt", false),
      artifact("beta/a-sub", true),
    ];
    const roots = buildTree(mixed);
    expect(names(roots)).toEqual(["alpha-dir", "beta", "alpha.txt", "zeta.txt"]);
    const beta = roots.find((n) => n.name === "beta");
    expect(names(beta?.children ?? [])).toEqual(["a-sub", "z.txt"]);
  });
});

describe("breadcrumbs", () => {
  it("starts at Root for the empty path", () => {
    expect(breadcrumbs("")).toEqual([{ label: "Root", path: "" }]);
  });

  it("builds cumulative crumb paths", () => {
    expect(breadcrumbs("mods/sub")).toEqual([
      { label: "Root", path: "" },
      { label: "mods", path: "mods" },
      { label: "sub", path: "mods/sub" },
    ]);
  });
});

describe("joinPath / parentPath", () => {
  it("joins relative to the root", () => {
    expect(joinPath("", "mods")).toBe("mods");
    expect(joinPath("mods", "a.jar")).toBe("mods/a.jar");
  });

  it("derives the parent folder", () => {
    expect(parentPath("a.jar")).toBe("");
    expect(parentPath("mods/a.jar")).toBe("mods");
    expect(parentPath("mods/sub/a.jar")).toBe("mods/sub");
  });
});

describe("segmentError", () => {
  it("accepts a plain single-segment name", () => {
    expect(segmentError("foo.jar")).toBeNull();
    expect(segmentError("my config")).toBeNull();
    expect(segmentError("a.b.c")).toBeNull();
  });

  it("rejects a name that would relocate the entry", () => {
    expect(segmentError("mods/foo.jar")).toMatch(/can’t contain/);
    expect(segmentError("mods\\foo.jar")).toMatch(/can’t contain/);
  });

  it("rejects the dot segments", () => {
    expect(segmentError(".")).toMatch(/can’t be/);
    expect(segmentError("..")).toMatch(/can’t be/);
    expect(segmentError(".hidden")).toBeNull();
  });
});
