import { Loader2 } from "lucide-react";
import type * as React from "react";
import { useEffect, useState } from "react";

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
import { joinPath, segmentError } from "@/features/builds/fileTree";
import { useCreateFolder } from "@/features/builds/api";

export function NewFolderDialog({
  slug,
  currentPath,
  open,
  onOpenChange,
}: {
  slug: string;
  currentPath: string;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const [name, setName] = useState("");
  const createFolder = useCreateFolder();

  const trimmed = name.trim();
  const nameError = trimmed === "" ? null : segmentError(trimmed);

  // The dialog stays mounted and the parent opens it bypassing Radix onOpenChange,
  // so reset the name here on open or a previously typed name lingers.
  useEffect(() => {
    if (open) {
      setName("");
    }
  }, [open]);

  function submit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!trimmed || nameError) {
      return;
    }
    createFolder.mutate(
      { slug, relativePath: joinPath(currentPath, trimmed) },
      { onSuccess: () => onOpenChange(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent>
        <form onSubmit={submit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>New folder</DialogTitle>
            <DialogDescription>
              Creates an empty folder in{" "}
              <code className="rounded-sm bg-surface-2 px-1 py-0.5 text-caption">
                {currentPath === "" ? "/" : currentPath}
              </code>
              .
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-1.5">
            <Label htmlFor="folder-name">Folder name</Label>
            <Input
              id="folder-name"
              value={name}
              autoFocus
              placeholder="config"
              aria-invalid={nameError !== null}
              aria-describedby={nameError ? "folder-name-error" : undefined}
              onChange={(event) => setName(event.target.value)}
            />
            {nameError ? (
              <p
                id="folder-name-error"
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
              disabled={createFolder.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createFolder.isPending || !trimmed || nameError !== null
              }
            >
              {createFolder.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Create folder
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
