import { ChevronRight, Folder, FolderRoot, Loader2 } from "lucide-react";
import { useState } from "react";

import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import type { FileTreeNode } from "@/features/builds/fileTree";
import { cn } from "@/shared/lib/cn";

interface FolderOption {
  relativePath: string;
  name: string;
  depth: number;
}

/// Flatten the tree to its folders (depth-first, folders only), so the dialog can
/// render an indented destination list.
function flattenFolders(nodes: FileTreeNode[], depth = 0): FolderOption[] {
  const out: FolderOption[] = [];
  for (const node of nodes) {
    if (!node.isDir) {
      continue;
    }
    out.push({ relativePath: node.relativePath, name: node.name, depth });
    out.push(...flattenFolders(node.children, depth + 1));
  }
  return out;
}

/// True when `targetDir` is one of the moved paths itself or a descendant — an
/// illegal destination (mirrors the server + DnD guard).
function isSelfOrDescendant(targetDir: string, sourcePaths: string[]): boolean {
  for (const src of sourcePaths) {
    if (targetDir === src || targetDir.startsWith(`${src}/`)) {
      return true;
    }
  }
  return false;
}

/// Destination-folder picker for the SelectionToolbar "Move" and the context-menu
/// "Move to…". Lists Root + every folder in the build; the source entries' own
/// subtree is disabled, as is each source's current parent (a no-op move).
export function MoveDialog({
  open,
  onOpenChange,
  roots,
  sourcePaths,
  pending,
  onMove,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  roots: FileTreeNode[];
  /// relativePaths of the entries being moved (for the self/descendant guard).
  sourcePaths: string[];
  pending: boolean;
  onMove: (targetDir: string) => void;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const folders = flattenFolders(roots);

  const options: FolderOption[] = [
    { relativePath: "", name: "Root", depth: 0 },
    ...folders,
  ];

  function disabledFor(path: string): boolean {
    return isSelfOrDescendant(path, sourcePaths);
  }

  return (
    <Dialog open={open} onOpenChange={(next) => !pending && onOpenChange(next)}>
      <DialogContent>
        <DialogHeader>
          <DialogTitle>Move to…</DialogTitle>
          <DialogDescription>
            Choose a destination folder for{" "}
            {sourcePaths.length === 1
              ? "the selected entry"
              : `${sourcePaths.length} entries`}
            .
          </DialogDescription>
        </DialogHeader>
        <ul className="mt-2 max-h-72 space-y-0.5 overflow-auto rounded-md border border-edge-md bg-surface-0 p-1.5">
          {options.map((option) => {
            const disabled = disabledFor(option.relativePath);
            const isSelected = selected === option.relativePath;
            return (
              <li key={option.relativePath || "__root__"}>
                <button
                  type="button"
                  disabled={disabled}
                  onClick={() => setSelected(option.relativePath)}
                  style={{ paddingLeft: `${option.depth * 1 + 0.5}rem` }}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-sm py-1.5 pr-2 text-left text-body outline-none transition-colors",
                    "hover:bg-ghost focus-visible:ring-2 focus-visible:ring-ring",
                    isSelected &&
                      "bg-accent-soft text-text-hi ring-1 ring-edge-xl",
                    disabled && "cursor-not-allowed opacity-40",
                  )}
                >
                  {option.relativePath === "" ? (
                    <FolderRoot className="size-4 shrink-0 text-text-mute" />
                  ) : (
                    <Folder className="size-4 shrink-0 text-text-mute" />
                  )}
                  <span className="truncate">{option.name}</span>
                  {isSelected ? (
                    <ChevronRight className="ml-auto size-4 text-text-faint" />
                  ) : null}
                </button>
              </li>
            );
          })}
        </ul>
        <DialogFooter className="mt-4">
          <Button
            type="button"
            variant="outline"
            disabled={pending}
            onClick={() => onOpenChange(false)}
          >
            Cancel
          </Button>
          <Button
            type="button"
            disabled={pending || selected === null}
            onClick={() => selected !== null && onMove(selected)}
          >
            {pending ? <Loader2 className="size-4 animate-spin" /> : null}
            Move here
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
