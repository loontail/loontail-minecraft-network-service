import { Download, FolderInput, Trash2, X } from "lucide-react";

import { Button } from "@/components/ui/button";

export function SelectionToolbar({
  count,
  onClear,
  onMove,
  onDownload,
  onDelete,
}: {
  count: number;
  onClear: () => void;
  onMove: () => void;
  onDownload: () => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2 rounded-md border border-edge-md bg-surface-1 px-3 py-2 text-body text-text">
      <Button
        variant="ghost"
        size="icon"
        className="size-7"
        aria-label="Clear selection"
        onClick={onClear}
      >
        <X className="size-4" />
      </Button>
      <span className="tabular-nums text-text-hi">{count} selected</span>
      <div className="ml-auto flex items-center gap-2">
        <Button variant="outline" size="sm" onClick={onMove}>
          <FolderInput className="size-4" />
          Move
        </Button>
        <Button variant="outline" size="sm" onClick={onDownload}>
          <Download className="size-4" />
          Download
        </Button>
        <Button variant="destructive" size="sm" onClick={onDelete}>
          <Trash2 className="size-4" />
          Delete
        </Button>
      </div>
    </div>
  );
}
