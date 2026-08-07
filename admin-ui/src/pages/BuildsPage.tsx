import {
  AlertCircle,
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
import { useNavigate } from "react-router";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { PageHeader } from "@/components/shared/PageHeader";
import {
  TableSkeletonRows,
  TableStateRow,
} from "@/components/shared/TableStates";
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
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useAdminBuilds,
  useCreateBuild,
  useDeleteBuild,
} from "@/features/builds/api";
import { errorMessage } from "@/shared/api/toast";
import type { BuildAdmin } from "@/shared/types";

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
  const createBuild = useCreateBuild();

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
    createBuild.mutate(
      {
        slug: trimmedSlug,
        available: false,
        locales: [{ locale: "en", title: trimmedTitle }],
      },
      {
        onSuccess: () => {
          handleOpenChange(false);
          // Navigate by the build slug, not the owned-bundle slug (which can differ and would 404).
          navigate(`/builds/${trimmedSlug}`);
        },
      },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <DialogContent>
        <form onSubmit={handleSubmit} className="flex flex-col gap-4">
          <DialogHeader>
            <DialogTitle>New build</DialogTitle>
            <DialogDescription>
              Builds are the modpacks shown in the launcher. We create the draft
              and its bundle, then open it so you can add details and files.
            </DialogDescription>
          </DialogHeader>
          <div className="space-y-4">
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
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => handleOpenChange(false)}
              disabled={createBuild.isPending}
            >
              Cancel
            </Button>
            <Button
              type="submit"
              disabled={
                createBuild.isPending || !slug.trim() || !title.trim()
              }
            >
              {createBuild.isPending ? (
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

export function BuildsPage() {
  const navigate = useNavigate();
  const builds = useAdminBuilds();
  const deleteBuild = useDeleteBuild();
  const [dialogOpen, setDialogOpen] = useState(false);
  const [confirm, setConfirm] = useState<BuildAdmin | null>(null);
  const rows = builds.data ?? [];
  const showEmpty = !builds.isLoading && !builds.isError && rows.length === 0;

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
            {builds.isLoading && <TableSkeletonRows columns={COLUMN_COUNT} />}

            {builds.isError && (
              <TableStateRow
                columns={COLUMN_COUNT}
                icon={AlertCircle}
                title="Could not load builds"
                description={errorMessage(builds.error, "Please try again.")}
              />
            )}

            {showEmpty && (
              <TableStateRow
                columns={COLUMN_COUNT}
                icon={Package}
                title="No builds yet"
                description="Create a build to surface a modpack in the launcher."
                action={
                  <Button
                    variant="outline"
                    onClick={() => setDialogOpen(true)}
                  >
                    <Plus className="size-4" />
                    New build
                  </Button>
                }
              />
            )}

            {!builds.isLoading &&
              !builds.isError &&
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
        pending={deleteBuild.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteBuild.mutate(confirm.id, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </div>
  );
}
