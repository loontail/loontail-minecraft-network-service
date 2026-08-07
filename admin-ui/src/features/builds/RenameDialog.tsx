import { Loader2 } from "lucide-react";
import type * as React from "react";
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
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  joinPath,
  parentPath,
  segmentError,
  type TreeEntry,
} from "@/features/builds/fileTree";
import { useRenameFile } from "@/features/builds/api";

export function RenameDialog({
  slug,
  entry,
  open,
  onOpenChange,
}: {
  slug: string;
  entry: TreeEntry;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState(entry.name);
  const rename = useRenameFile();

  const trimmed = name.trim();
  const nameError = trimmed === "" ? null : segmentError(trimmed);

  function handleOpenChange(next: boolean) {
    if (next) {
      setName(entry.name);
    }
    onOpenChange(next);
  }

  function submit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!trimmed || trimmed === entry.name || nameError || !entry.artifact) {
      return;
    }
    const newRelativePath = joinPath(parentPath(entry.relativePath), trimmed);
    rename.mutate(
      { slug, entryId: entry.artifact.id, newRelativePath },
      { onSuccess: () => onOpenChange(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>Rename “{entry.name}”</DialogTitle>
            <DialogDescription>
              Renames the {entry.isDir ? "folder" : "file"} within its current
              folder.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="rename-name">New name</Label>
            <Input
              id="rename-name"
              value={name}
              autoFocus
              aria-invalid={nameError !== null}
              aria-describedby={nameError ? "rename-name-error" : undefined}
              onChange={(event) => setName(event.target.value)}
            />
            {nameError ? (
              <p
                id="rename-name-error"
                className="text-caption text-destructive"
              >
                {nameError}
              </p>
            ) : null}
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={rename.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                rename.isPending ||
                !trimmed ||
                trimmed === entry.name ||
                nameError !== null
              }
            >
              {rename.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Rename
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
