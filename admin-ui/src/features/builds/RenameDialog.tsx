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
import { joinPath, parentPath, type TreeEntry } from "@/features/builds/fileTree";
import { useRenameFile } from "@/features/bundles/api";

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

  function handleOpenChange(next: boolean) {
    if (next) {
      setName(entry.name);
    }
    onOpenChange(next);
  }

  function submit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed || trimmed === entry.name || !entry.artifact) {
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
        <form onSubmit={submit}>
          <DialogHeader>
            <DialogTitle>Rename “{entry.name}”</DialogTitle>
            <DialogDescription>
              Renames the {entry.isDir ? "folder" : "file"} within its current
              folder.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 space-y-1.5">
            <Label htmlFor="rename-name">New name</Label>
            <Input
              id="rename-name"
              value={name}
              autoFocus
              onChange={(event) => setName(event.target.value)}
            />
          </div>
          <DialogFooter className="mt-6">
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
                rename.isPending || !name.trim() || name.trim() === entry.name
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
