import {
  ArrowRight,
  EyeOff,
  Loader2,
  Package,
  Pencil,
  Plus,
  Trash2,
} from "lucide-react";
import type * as React from "react";
import { useState } from "react";
import { useNavigate } from "react-router-dom";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { PageHeader } from "@/components/shared/PageHeader";
import { Badge } from "@/components/ui/badge";
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
import { Skeleton } from "@/components/ui/skeleton";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useAdminClients,
  useCreateClient,
  useDeleteClient,
} from "@/features/builds/api";
import type { ClientAdmin } from "@/shared/types";

const COLUMN_COUNT = 6;

function CreateBuildDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const [slug, setSlug] = useState("");
  const [title, setTitle] = useState("");
  const createClient = useCreateClient();

  function handleOpenChange(next: boolean) {
    if (next) {
      setSlug("");
      setTitle("");
    }
    onOpenChange(next);
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    const trimmedSlug = slug.trim();
    const trimmedTitle = title.trim();
    if (!trimmedSlug || !trimmedTitle) {
      return;
    }
    createClient.mutate(
      {
        slug: trimmedSlug,
        available: false,
        locales: [{ locale: "en", title: trimmedTitle }],
      },
      {
        onSuccess: (res) => {
          handleOpenChange(false);
          navigate(`/builds/${res.bundleSlug ?? trimmedSlug}`);
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>New build</DialogTitle>
            <DialogDescription>
              Builds are the modpacks shown in the launcher. We create the draft
              and its bundle, then open it so you can add details and files.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 space-y-4">
            <div className="space-y-1.5">
              <Label htmlFor="build-slug">Slug</Label>
              <Input
                id="build-slug"
                value={slug}
                placeholder="all-the-mods-9"
                onChange={(event) => setSlug(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="build-title">Title</Label>
              <Input
                id="build-title"
                value={title}
                placeholder="All the Mods 9"
                onChange={(event) => setTitle(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter className="mt-6">
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
              disabled={createClient.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createClient.isPending || !slug.trim() || !title.trim()
              }
            >
              {createClient.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Create build
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}

function SkeletonRows() {
  return (
    <>
      {Array.from({ length: 6 }).map((_, index) => (
        // biome-ignore lint/suspicious/noArrayIndexKey: static placeholder rows
        <TableRow key={index}>
          <TableCell>
            <Skeleton className="h-4 w-40" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-32" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-16" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-4 w-8" />
          </TableCell>
          <TableCell>
            <Skeleton className="h-5 w-24" />
          </TableCell>
          <TableCell>
            <Skeleton className="ml-auto h-8 w-20" />
          </TableCell>
        </TableRow>
      ))}
    </>
  );
}

export function BuildsPage() {
  const navigate = useNavigate();
  const builds = useAdminClients();
  const deleteClient = useDeleteClient();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirm, setConfirm] = useState<ClientAdmin | null>(null);
  const rows = builds.data ?? [];
  const showEmpty = !builds.isLoading && rows.length === 0;

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Builds"
        description="Modpacks shown in the launcher — details, media, and bundled files."
        actions={
          <Button onClick={() => setDialogOpen(true)}>
            <Plus className="size-4" />
            New build
          </Button>
        }
      />
      <div className="overflow-hidden rounded-lg border border-edge bg-card">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead>Title</TableHead>
              <TableHead>Slug</TableHead>
              <TableHead>Minecraft</TableHead>
              <TableHead>Files</TableHead>
              <TableHead>State</TableHead>
              <TableHead className="w-px text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {builds.isLoading && <SkeletonRows />}

            {showEmpty && (
              <TableRow className="hover:bg-transparent">
                <TableCell colSpan={COLUMN_COUNT} className="h-48 text-center">
                  <div className="flex flex-col items-center justify-center gap-2 text-text-mute">
                    <Package className="size-8 text-text-faint" />
                    <p className="text-body-med text-text-hi">No builds yet</p>
                    <p className="text-caption">
                      Create a build to surface a modpack in the launcher.
                    </p>
                    <Button
                      variant="outline"
                      className="mt-2"
                      onClick={() => setDialogOpen(true)}
                    >
                      <Plus className="size-4" />
                      New build
                    </Button>
                  </div>
                </TableCell>
              </TableRow>
            )}

            {!builds.isLoading &&
              rows.map((build) => (
                <TableRow key={build.id}>
                  <TableCell className="font-medium text-text-hi">
                    {build.title}
                  </TableCell>
                  <TableCell className="text-text-mute">{build.slug}</TableCell>
                  <TableCell className="text-text-mute">
                    {build.minecraftVersion ?? "—"}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {build.bundle?.filesCount ?? 0}
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center gap-2">
                      {build.published ? (
                        <Badge variant="outline">Published</Badge>
                      ) : (
                        <Badge variant="secondary">
                          <Pencil className="size-3" />
                          Draft
                        </Badge>
                      )}
                      {build.available ? null : (
                        <Badge variant="outline" className="text-text-faint">
                          <EyeOff className="size-3" />
                          Hidden
                        </Badge>
                      )}
                    </div>
                  </TableCell>
                  <TableCell>
                    <div className="flex items-center justify-end gap-1">
                      <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => navigate(`/builds/${build.slug}`)}
                      >
                        Open
                        <ArrowRight className="size-4" />
                      </Button>
                      <Button
                        variant="ghost"
                        size="icon"
                        aria-label={`Delete ${build.title}`}
                        onClick={() => setConfirm(build)}
                      >
                        <Trash2 className="size-4" />
                      </Button>
                    </div>
                  </TableCell>
                </TableRow>
              ))}
          </TableBody>
        </Table>
      </div>

      <CreateBuildDialog open={dialogOpen} onOpenChange={setDialogOpen} />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(next) => !next && setConfirm(null)}
        title={`Delete “${confirm?.title}”?`}
        description="This permanently deletes the build, its bundle, and all of its files. This cannot be undone."
        confirmLabel="Delete build"
        destructive
        pending={deleteClient.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteClient.mutate(confirm.id, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </div>
  );
}
