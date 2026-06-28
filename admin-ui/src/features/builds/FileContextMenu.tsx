import {
  Download,
  FolderInput,
  FolderOpen,
  Hash,
  Pencil,
  Trash2,
  ZapOff,
} from "lucide-react";

import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import type { TreeEntry } from "@/features/builds/fileTree";

/// What `onAction` reports back to the orchestrator; each maps to a reused mutation
/// or navigation. Folder-only / file-only items are filtered out before render.
export type FileMenuAction =
  | "open"
  | "download"
  | "rename"
  | "move"
  | "toggle-once"
  | "rehash"
  | "delete";

export interface FileMenuTarget {
  /// Screen-space coordinates the menu opens at: the cursor for a right-click, or the
  /// ⋮ button's rect for a click. A single cursor-anchored menu serves both.
  position: { x: number; y: number };
  entry: TreeEntry;
}

/// The single Google-Drive-style context menu, shared by the per-card ⋮ button and
/// the right-click handler. Radix has no virtual-anchor prop, so a zero-size
/// fixed-positioned trigger is rendered at `target.position` and the Radix menu opens
/// under it — identical behaviour whether the open came from a right-click or the ⋮.
///
/// Implied folders (`artifact === null`) disable every mutating item — they have no
/// backing row to rename / move / delete / rehash.
export function FileContextMenu({
  open,
  onOpenChange,
  target,
  onAction,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  target: FileMenuTarget | null;
  onAction: (action: FileMenuAction, entry: TreeEntry) => void;
}) {
  if (!target) {
    return null;
  }

  const { entry, position } = target;
  const isFile = !entry.isDir;
  const mutable = entry.artifact !== null;

  function act(action: FileMenuAction) {
    onAction(action, entry);
    onOpenChange(false);
  }

  return (
    <DropdownMenu open={open} onOpenChange={onOpenChange}>
      <DropdownMenuTrigger
        aria-hidden
        tabIndex={-1}
        style={{
          position: "fixed",
          left: position.x,
          top: position.y,
          width: 0,
          height: 0,
        }}
      />
      <DropdownMenuContent align="start" className="min-w-44">
        <DropdownMenuItem onSelect={() => act("open")}>
          {entry.isDir ? (
            <FolderOpen className="size-4" />
          ) : (
            <Download className="size-4" />
          )}
          Open
        </DropdownMenuItem>
        {isFile ? (
          <DropdownMenuItem onSelect={() => act("download")}>
            <Download className="size-4" />
            Download
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuItem disabled={!mutable} onSelect={() => act("rename")}>
          <Pencil className="size-4" />
          Rename
        </DropdownMenuItem>
        <DropdownMenuItem disabled={!mutable} onSelect={() => act("move")}>
          <FolderInput className="size-4" />
          Move to…
        </DropdownMenuItem>
        {isFile ? (
          <DropdownMenuItem
            disabled={!mutable}
            onSelect={() => act("toggle-once")}
          >
            <ZapOff className="size-4" />
            {entry.artifact?.downloadOnce
              ? "Disable download-once"
              : "Enable download-once"}
          </DropdownMenuItem>
        ) : null}
        {isFile ? (
          <DropdownMenuItem disabled={!mutable} onSelect={() => act("rehash")}>
            <Hash className="size-4" />
            Rehash
          </DropdownMenuItem>
        ) : null}
        <DropdownMenuSeparator />
        <DropdownMenuItem
          variant="destructive"
          disabled={!mutable}
          onSelect={() => act("delete")}
        >
          <Trash2 className="size-4" />
          Delete
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
