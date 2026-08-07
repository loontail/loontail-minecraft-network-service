import type { BundleArtifact } from "@/shared/types";

export interface TreeEntry {
  relativePath: string;
  name: string;
  isDir: boolean;
  // null for folders implied only by a descendant file (no explicit dir row).
  artifact: BundleArtifact | null;
}

export function joinPath(base: string, segment: string): string {
  return base === "" ? segment : `${base}/${segment}`;
}

// why: the name dialogs edit ONE segment, but they hand the result to joinPath and
// the backend accepts any relative path — so "mods/foo.jar" in the rename box is a
// silent move out of the current folder, reported as "Renamed". Reject separators
// client-side; the server has no rule that would.
export function segmentError(name: string): string | null {
  if (name.includes("/") || name.includes("\\")) {
    return "Name can’t contain “/” or “\\”.";
  }
  if (name === "." || name === "..") {
    return "Name can’t be “.” or “..”.";
  }
  return null;
}

export function parentPath(relativePath: string): string {
  const idx = relativePath.lastIndexOf("/");
  return idx === -1 ? "" : relativePath.slice(0, idx);
}

function segments(path: string): string[] {
  return path.split("/").filter((part) => part !== "");
}

// Immediate children of `currentPath`. Explicit dir rows win over folders only
// implied by a deeper file, which are synthesized so navigation never dead-ends.
export function childrenOf(
  artifacts: BundleArtifact[],
  currentPath: string,
): TreeEntry[] {
  const prefix = currentPath === "" ? "" : `${currentPath}/`;
  const depth = segments(currentPath).length;

  const explicit = new Map<string, TreeEntry>();
  const implied = new Map<string, TreeEntry>();

  for (const artifact of artifacts) {
    if (currentPath !== "" && !artifact.relativePath.startsWith(prefix)) {
      continue;
    }
    if (artifact.relativePath === currentPath) {
      continue;
    }
    const parts = segments(artifact.relativePath);
    const childName = parts[depth];
    if (childName === undefined) {
      continue;
    }
    const childPath = joinPath(currentPath, childName);

    if (parts.length === depth + 1) {
      explicit.set(childPath, {
        relativePath: childPath,
        name: childName,
        isDir: artifact.isDir,
        artifact,
      });
    } else if (!implied.has(childPath)) {
      implied.set(childPath, {
        relativePath: childPath,
        name: childName,
        isDir: true,
        artifact: null,
      });
    }
  }

  const merged = new Map<string, TreeEntry>(implied);
  for (const [path, entry] of explicit) {
    merged.set(path, entry);
  }

  return [...merged.values()].sort((a, b) => {
    if (a.isDir !== b.isDir) {
      return a.isDir ? -1 : 1;
    }
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  });
}

export interface FileTreeNode {
  // Stable react-aria / drag-and-drop key === the node's relativePath.
  id: string;
  relativePath: string;
  name: string;
  isDir: boolean;
  artifact: BundleArtifact | null;
  children: FileTreeNode[];
}

export function buildTree(artifacts: BundleArtifact[]): FileTreeNode[] {
  const nodes = new Map<string, FileTreeNode>();

  function ensure(relativePath: string, isDir: boolean): FileTreeNode {
    const existing = nodes.get(relativePath);
    if (existing) {
      return existing;
    }
    const parts = segments(relativePath);
    const node: FileTreeNode = {
      id: relativePath,
      relativePath,
      name: parts[parts.length - 1] ?? relativePath,
      isDir,
      artifact: null,
      children: [],
    };
    nodes.set(relativePath, node);
    const parent = parentPath(relativePath);
    if (parent !== "") {
      ensure(parent, true).children.push(node);
    }
    return node;
  }

  const roots: FileTreeNode[] = [];

  for (const artifact of artifacts) {
    const parent = parentPath(artifact.relativePath);
    if (parent !== "") {
      ensure(parent, true);
    }
  }

  // Explicit artifact rows must win over the implied folders created above.
  for (const artifact of artifacts) {
    const node = ensure(artifact.relativePath, artifact.isDir);
    node.isDir = artifact.isDir;
    node.artifact = artifact;
  }

  for (const node of nodes.values()) {
    if (parentPath(node.relativePath) === "") {
      roots.push(node);
    }
  }

  const byName = (a: FileTreeNode, b: FileTreeNode) => {
    if (a.isDir !== b.isDir) {
      return a.isDir ? -1 : 1;
    }
    return a.name.localeCompare(b.name, undefined, { sensitivity: "base" });
  };

  function sortRec(list: FileTreeNode[]) {
    list.sort(byName);
    for (const node of list) {
      sortRec(node.children);
    }
  }
  sortRec(roots);

  return roots;
}

export interface Crumb {
  label: string;
  path: string;
}

export function breadcrumbs(currentPath: string): Crumb[] {
  const crumbs: Crumb[] = [{ label: "Root", path: "" }];
  let acc = "";
  for (const part of segments(currentPath)) {
    acc = joinPath(acc, part);
    crumbs.push({ label: part, path: acc });
  }
  return crumbs;
}
