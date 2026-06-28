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
import { joinPath } from "@/features/builds/fileTree";
import { useCreateFolder } from "@/features/bundles/api";

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

  // The dialog stays mounted and the parent opens it by setting state directly
  // (bypassing Radix onOpenChange), so reset the name on every open here rather
  // than relying on handleOpenChange — otherwise a previously typed name lingers.
  useEffect(() => {
    if (open) {
      setName("");
    }
  }, [open]);

  function submit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmed = name.trim();
    if (!trimmed) {
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
              onChange={(event) => setName(event.target.value)}
            />
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
            <Button type="submit" disabled={createFolder.isPending || !name.trim()}>
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
