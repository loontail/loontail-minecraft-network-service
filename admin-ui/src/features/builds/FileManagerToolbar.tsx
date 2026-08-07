import {
  FileArchive,
  FolderPlus,
  LayoutGrid,
  List as ListIcon,
  Loader2,
  MoreHorizontal,
  Plus,
  RefreshCw,
  ShieldCheck,
  Upload,
} from "lucide-react";
import type * as React from "react";
import { useRef } from "react";

import { Button } from "@/components/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { cn } from "@/shared/lib/cn";

export type ViewMode = "grid" | "list";

export function FileManagerToolbar({
  viewMode,
  onViewModeChange,
  onNewFolder,
  onUploadFiles,
  onUploadArchive,
  onRegenerate,
  onValidate,
  uploadPending,
  archivePending,
  regeneratePending,
  validatePending,
}: {
  viewMode: ViewMode;
  onViewModeChange: (mode: ViewMode) => void;
  onNewFolder: () => void;
  onUploadFiles: (files: FileList) => void;
  onUploadArchive: (file: File) => void;
  onRegenerate: () => void;
  onValidate: () => void;
  uploadPending: boolean;
  archivePending: boolean;
  regeneratePending: boolean;
  validatePending: boolean;
}) {
  const fileInputRef = useRef<HTMLInputElement>(null);
  const zipInputRef = useRef<HTMLInputElement>(null);

  function onFilePicked(event: React.ChangeEvent<HTMLInputElement>) {
    const files = event.target.files;
    if (files && files.length > 0) {
      onUploadFiles(files);
    }
    event.target.value = "";
  }

  function onZipPicked(event: React.ChangeEvent<HTMLInputElement>) {
    const file = event.target.files?.[0];
    if (file) {
      onUploadArchive(file);
    }
    event.target.value = "";
  }

  const busy = uploadPending || archivePending;

  return (
    <div className="flex flex-wrap items-center gap-2">
      <input
        ref={fileInputRef}
        type="file"
        multiple
        hidden
        onChange={onFilePicked}
      />
      <input
        ref={zipInputRef}
        type="file"
        accept=".zip"
        hidden
        onChange={onZipPicked}
      />

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <Button size="sm" disabled={busy}>
            {busy ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Plus className="size-4" />
            )}
            New
          </Button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="min-w-44">
          <DropdownMenuItem onSelect={onNewFolder}>
            <FolderPlus className="size-4" />
            New folder
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem onSelect={() => fileInputRef.current?.click()}>
            <Upload className="size-4" />
            Upload file
          </DropdownMenuItem>
          <DropdownMenuItem onSelect={() => zipInputRef.current?.click()}>
            <FileArchive className="size-4" />
            Upload ZIP
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>

      <div className="ml-auto flex items-center gap-2">
        <div
          role="group"
          aria-label="View mode"
          className="flex items-center rounded-md border border-edge-md bg-surface-1 p-0.5"
        >
          <Button
            variant="ghost"
            size="icon"
            aria-label="Grid view"
            aria-pressed={viewMode === "grid"}
            className={cn(
              "size-7",
              viewMode === "grid" && "bg-surface-3 text-text-hi",
            )}
            onClick={() => onViewModeChange("grid")}
          >
            <LayoutGrid className="size-4" />
          </Button>
          <Button
            variant="ghost"
            size="icon"
            aria-label="List view"
            aria-pressed={viewMode === "list"}
            className={cn(
              "size-7",
              viewMode === "list" && "bg-surface-3 text-text-hi",
            )}
            onClick={() => onViewModeChange("list")}
          >
            <ListIcon className="size-4" />
          </Button>
        </div>

        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <Button
              variant="outline"
              size="icon"
              className="size-8"
              aria-label="More actions"
            >
              <MoreHorizontal className="size-4" />
            </Button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="min-w-44">
            <DropdownMenuItem
              disabled={regeneratePending}
              onSelect={onRegenerate}
            >
              {regeneratePending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <RefreshCw className="size-4" />
              )}
              Regenerate
            </DropdownMenuItem>
            <DropdownMenuItem disabled={validatePending} onSelect={onValidate}>
              {validatePending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <ShieldCheck className="size-4" />
              )}
              Validate
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>
      </div>
    </div>
  );
}
