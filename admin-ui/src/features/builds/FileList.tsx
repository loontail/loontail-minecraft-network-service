import { File as FileIcon, Folder, MoreVertical } from "lucide-react";
import type { PointerEvent as ReactPointerEvent } from "react";
import { useRef } from "react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Switch } from "@/components/ui/switch";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import type { TreeEntry } from "@/features/builds/fileTree";
import { cn } from "@/shared/lib/cn";
import { formatBytes } from "@/shared/lib/format";

// Shared by the list and grid views. Folders carry no artifact id, so only
// artifact-backed entries are selectable.
export interface FileViewProps {
  entries: TreeEntry[];
  selectedKeys: Set<string>;
  onToggle: (relativePath: string, shiftKey: boolean) => void;
  onToggleAll: (checked: boolean) => void;
  allSelected: boolean;
  someSelected: boolean;
  onAction: (relativePath: string) => void;
  onOpenMenu: (entry: TreeEntry, position: { x: number; y: number }) => void;
  onToggleDownloadOnce: (entry: TreeEntry) => void;
}

function ShaCell({ sha256 }: { sha256: string | null }) {
  if (!sha256) {
    return <span className="text-text-faint">—</span>;
  }
  return (
    <span
      className="font-mono text-caption text-text-mute"
      title={sha256}
    >
      {sha256.slice(0, 6)}…{sha256.slice(-6)}
    </span>
  );
}

// why: reveal the checkbox/kebab only on hover where hover exists; on coarse
// pointers (touch) and selected rows it stays visible.
export const REVEAL =
  "[@media(hover:hover)]:opacity-0 [@media(hover:hover)]:group-hover:opacity-100 [@media(hover:hover)]:group-data-[state=selected]:opacity-100 [@media(hover:hover)]:group-focus-within:opacity-100";

export function useLongPress(
  onLongPress: (position: { x: number; y: number }) => void,
) {
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null);

  function clear() {
    if (timer.current) {
      clearTimeout(timer.current);
      timer.current = null;
    }
  }

  return {
    onPointerDown(e: ReactPointerEvent) {
      if (e.pointerType === "mouse") {
        return;
      }
      const { clientX: x, clientY: y } = e;
      clear();
      timer.current = setTimeout(() => onLongPress({ x, y }), 500);
    },
    onPointerUp: clear,
    onPointerMove: clear,
    onPointerCancel: clear,
  };
}

export function FileList({
  entries,
  selectedKeys,
  onToggle,
  onToggleAll,
  allSelected,
  someSelected,
  onAction,
  onOpenMenu,
  onToggleDownloadOnce,
}: FileViewProps) {
  return (
    <Table aria-label="Build files">
      <TableHeader>
        <TableRow>
          <TableHead className="w-10">
            <button
              type="button"
              aria-label="Select all"
              className="flex items-center justify-center"
              onClick={() => onToggleAll(!allSelected)}
            >
              <Checkbox
                checked={allSelected}
                className={someSelected ? "opacity-60" : undefined}
              />
            </button>
          </TableHead>
          <TableHead>Name</TableHead>
          <TableHead className="w-28 text-right">Size</TableHead>
          <TableHead className="w-40">SHA-256</TableHead>
          <TableHead className="w-28 text-right">Download once</TableHead>
          <TableHead className="w-12" aria-label="Actions" />
        </TableRow>
      </TableHeader>
      <TableBody>
        {entries.map((entry) => (
          <FileRow
            key={entry.relativePath}
            entry={entry}
            selected={selectedKeys.has(entry.relativePath)}
            onToggle={onToggle}
            onAction={onAction}
            onOpenMenu={onOpenMenu}
            onToggleDownloadOnce={onToggleDownloadOnce}
          />
        ))}
      </TableBody>
    </Table>
  );
}

function FileRow({
  entry,
  selected,
  onToggle,
  onAction,
  onOpenMenu,
  onToggleDownloadOnce,
}: {
  entry: TreeEntry;
  selected: boolean;
  onToggle: (relativePath: string, shiftKey: boolean) => void;
  onAction: (relativePath: string) => void;
  onOpenMenu: (entry: TreeEntry, position: { x: number; y: number }) => void;
  onToggleDownloadOnce: (entry: TreeEntry) => void;
}) {
  const longPress = useLongPress((position) => onOpenMenu(entry, position));
  const selectable = entry.artifact !== null && !entry.isDir;

  return (
    <TableRow
      className={cn(
        "group",
        "data-[state=selected]:shadow-[inset_2px_0_0] data-[state=selected]:shadow-primary",
      )}
      data-state={selected ? "selected" : undefined}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenMenu(entry, { x: e.clientX, y: e.clientY });
      }}
      {...longPress}
    >
      <TableCell className="w-10">
        {entry.artifact ? (
          <button
            type="button"
            aria-label={`Select ${entry.name}`}
            className={cn(
              "flex items-center justify-center transition-opacity",
              !selected && REVEAL,
            )}
            onClick={(e) => {
              e.stopPropagation();
              onToggle(entry.relativePath, e.shiftKey);
            }}
          >
            <Checkbox checked={selected} />
          </button>
        ) : null}
      </TableCell>
      <TableCell>
        <button
          type="button"
          className={cn(
            "flex min-w-0 items-center gap-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-sm",
            entry.isDir
              ? "text-text-hi cursor-pointer hover:underline"
              : "text-text-hi",
          )}
          onClick={() => onAction(entry.relativePath)}
        >
          {entry.isDir ? (
            <Folder className="size-4 shrink-0 text-text-mute" />
          ) : (
            <FileIcon className="size-4 shrink-0 text-text-faint" />
          )}
          <span className="truncate text-body" title={entry.name}>
            {entry.name}
          </span>
        </button>
      </TableCell>
      <TableCell className="w-28 text-right text-caption text-text-mute tabular-nums">
        {entry.isDir || !entry.artifact ? "" : formatBytes(entry.artifact.size)}
      </TableCell>
      <TableCell className="w-40">
        {entry.isDir ? null : <ShaCell sha256={entry.artifact?.sha256 ?? null} />}
      </TableCell>
      <TableCell className="w-28 text-right">
        {selectable && entry.artifact ? (
          <Switch
            checked={entry.artifact.downloadOnce}
            aria-label={`Download once for ${entry.name}`}
            onCheckedChange={() => onToggleDownloadOnce(entry)}
          />
        ) : null}
      </TableCell>
      <TableCell className="w-12">
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Actions for ${entry.name}`}
          className={cn(
            "size-7 shrink-0 transition-opacity",
            REVEAL,
          )}
          onClick={(e) => {
            e.stopPropagation();
            const rect = e.currentTarget.getBoundingClientRect();
            onOpenMenu(entry, { x: rect.left, y: rect.bottom });
          }}
        >
          <MoreVertical className="size-4" />
        </Button>
      </TableCell>
    </TableRow>
  );
}
