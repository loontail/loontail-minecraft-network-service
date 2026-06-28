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

export type FileMenuAction =
  | "open"
  | "download"
  | "rename"
  | "move"
  | "toggle-once"
  | "rehash"
  | "delete";

export interface FileMenuTarget {
  position: { x: number; y: number };
  entry: TreeEntry;
}

// why: Radix has no virtual-anchor prop, so a zero-size fixed trigger is rendered
// at `target.position` and the menu opens under it.
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
