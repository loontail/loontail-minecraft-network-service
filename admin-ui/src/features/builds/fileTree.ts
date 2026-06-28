import type { BundleArtifact } from "@/shared/types";

/// A single row in the nested folder view: either a real artifact entry or a
/// folder implied by a child file path (when the backend did not emit an explicit
/// dir row). Implied folders carry no `id`/`size`/`sha256` of their own.
export interface TreeEntry {
  /// Forward-slash relative path of this entry (e.g. "mods/a.jar").
  relativePath: string;
  /// The last path segment (display name).
  name: string;
  isDir: boolean;
  /// The backing artifact row, present for explicit entries. Implied folders
  /// (derived only from a descendant file) have no artifact.
  artifact: BundleArtifact | null;
}

/// Join a folder path and a child segment with a single forward slash, treating
/// "" as the root.
export function joinPath(base: string, segment: string): string {
  return base === "" ? segment : `${base}/${segment}`;
}

/// The parent folder path of a relative path ("" for a top-level entry).
export function parentPath(relativePath: string): string {
  const idx = relativePath.lastIndexOf("/");
  return idx === -1 ? "" : relativePath.slice(0, idx);
}

/// Split a relative path into non-empty segments.
function segments(path: string): string[] {
  return path.split("/").filter((part) => part !== "");
}

/// Compute the immediate children of `currentPath` from the flat artifact list.
///
/// An entry is an immediate child when its path, relative to `currentPath`, has
/// exactly one segment. Explicit dir rows are preferred; folders only implied by a
/// deeper file (no explicit row) are synthesized so navigation never dead-ends.
/// Folders sort before files, each group alphabetically (case-insensitive).
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

/// A node in the nested tree consumed by the react-aria `Tree`. The `id` is the
/// node's `relativePath` (the stable key the Tree and drag-and-drop operate on).
/// `artifact` is the backing row when the node has an explicit artifact; implied
/// folders (derived only from a descendant file) have `artifact: null` and are NOT
/// draggable / movable on their own.
export interface FileTreeNode {
  /// Stable react-aria key === the node's forward-slash relative path.
  id: string;
  relativePath: string;
  name: string;
  isDir: boolean;
  artifact: BundleArtifact | null;
  children: FileTreeNode[];
}

/// Build the nested tree for the react-aria `Tree` from the flat artifact list.
///
/// Every artifact contributes its own node and, where needed, implied ancestor
/// folders so a deep file never dead-ends. Explicit dir rows are preferred over the
/// folder a child path would imply (so the dir keeps its artifact `id`). Children of
/// every node are sorted folders-first, then alphabetically (case-insensitive).
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

  // First pass: create implied ancestor folders so every node has its chain.
  for (const artifact of artifacts) {
    const parent = parentPath(artifact.relativePath);
    if (parent !== "") {
      ensure(parent, true);
    }
  }

  // Second pass: attach the explicit artifact rows (these win over implied folders).
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

/// Breadcrumb crumbs for `currentPath`, root first. Each crumb's `path` is the
/// folder to navigate to when clicked.
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
