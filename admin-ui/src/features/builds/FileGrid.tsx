import { File as FileIcon, Folder, MoreVertical } from "lucide-react";

import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import {
  type FileViewProps,
  REVEAL,
  useLongPress,
} from "@/features/builds/FileList";
import type { TreeEntry } from "@/features/builds/fileTree";
import { cn } from "@/shared/lib/cn";
import { formatBytes } from "@/shared/lib/format";

// Compact cards have no inline download-once toggle (unlike the list's Switch
// column); it is reached via the context menu, so `onToggleDownloadOnce` is unused.
export function FileGrid({
  entries,
  selectedKeys,
  onToggle,
  onAction,
  onOpenMenu,
}: FileViewProps) {
  return (
    <div className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
      {entries.map((entry) => (
        <FileCard
          key={entry.relativePath}
          entry={entry}
          selected={selectedKeys.has(entry.relativePath)}
          onToggle={onToggle}
          onAction={onAction}
          onOpenMenu={onOpenMenu}
        />
      ))}
    </div>
  );
}

function FileCard({
  entry,
  selected,
  onToggle,
  onAction,
  onOpenMenu,
}: {
  entry: TreeEntry;
  selected: boolean;
  onToggle: (relativePath: string, shiftKey: boolean) => void;
  onAction: (relativePath: string) => void;
  onOpenMenu: (entry: TreeEntry, position: { x: number; y: number }) => void;
}) {
  const longPress = useLongPress((position) => onOpenMenu(entry, position));

  return (
    <div
      className={cn(
        "group relative flex flex-col gap-2 rounded-lg border bg-surface-1 p-3 transition-colors",
        selected
          ? "border-primary bg-accent-soft ring-1 ring-primary"
          : "border-edge-md hover:bg-surface-2",
      )}
      data-state={selected ? "selected" : undefined}
      onContextMenu={(e) => {
        e.preventDefault();
        onOpenMenu(entry, { x: e.clientX, y: e.clientY });
      }}
      {...longPress}
    >
      <button
        type="button"
        aria-label={entry.name}
        className="flex min-w-0 flex-col gap-2 text-left outline-none focus-visible:ring-2 focus-visible:ring-ring rounded-md"
        onClick={() => onAction(entry.relativePath)}
      >
        {entry.isDir ? (
          <Folder className="size-7 shrink-0 text-text-mute" />
        ) : (
          <FileIcon className="size-7 shrink-0 text-text-faint" />
        )}
        <div className="min-w-0 space-y-0.5">
          <p
            className={cn(
              "truncate text-body text-text-hi",
              entry.isDir && "hover:underline",
            )}
            title={entry.name}
          >
            {entry.name}
          </p>
          <p className="text-caption text-text-faint tabular-nums">
            {entry.isDir
              ? "Folder"
              : entry.artifact
                ? formatBytes(entry.artifact.size)
                : "—"}
          </p>
        </div>
      </button>

      {entry.artifact ? (
        <button
          type="button"
          aria-label={`Select ${entry.name}`}
          className={cn(
            "absolute left-2 top-2 flex items-center justify-center transition-opacity",
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

      <Button
        variant="ghost"
        size="icon"
        aria-label={`Actions for ${entry.name}`}
        className={cn("absolute right-1 top-1 size-7 transition-opacity", REVEAL)}
        onClick={(e) => {
          e.stopPropagation();
          const rect = e.currentTarget.getBoundingClientRect();
          onOpenMenu(entry, { x: rect.left, y: rect.bottom });
        }}
      >
        <MoreVertical className="size-4" />
      </Button>
    </div>
  );
}
