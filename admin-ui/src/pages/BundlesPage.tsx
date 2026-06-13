import {
  AlertTriangle,
  Boxes,
  CheckCircle2,
  Clock,
  ExternalLink,
  FileArchive,
  FileText,
  Folder,
  HardDrive,
  Loader2,
  Plus,
  RefreshCw,
  ShieldCheck,
  Trash2,
  Upload,
} from "lucide-react";
import type * as React from "react";
import { useRef, useState } from "react";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { EmptyState } from "@/components/shared/EmptyState";
import { PageHeader } from "@/components/shared/PageHeader";
import { TableSkeleton } from "@/components/shared/TableSkeleton";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
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
import { Separator } from "@/components/ui/separator";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  useBuild,
  useBuilds,
  useCreateBuild,
  useDeleteBuild,
  useDeleteFile,
  useDiskSpace,
  useRegenerateManifest,
  useUploadArchive,
  useValidateBuild,
} from "@/features/bundles/api";
import { formatBytes, formatDateTime } from "@/shared/lib/format";
import type { Bundle, BundleArtifact, ValidateResult } from "@/shared/types";

function manifestUrl(slug: string): string {
  return `/api/bundle-registry/builds/${slug}/manifest`;
}

function StatusBadge({ bundle }: { bundle: Bundle }) {
  const status = bundle.status;
  if (status === "ready") {
    return (
      <Badge variant="outline">
        <CheckCircle2 className="size-3" />
        Ready
      </Badge>
    );
  }
  if (status === "failed") {
    return (
      <Badge variant="destructive">
        <AlertTriangle className="size-3" />
        Failed
      </Badge>
    );
  }
  if (status === "processing") {
    return (
      <Badge variant="secondary">
        <Loader2 className="size-3 animate-spin" />
        Processing
      </Badge>
    );
  }
  return (
    <Badge variant="secondary">
      <Clock className="size-3" />
      Draft
    </Badge>
  );
}

function CreateBuildDialog() {
  const [open, setOpen] = useState(false);
  const [name, setName] = useState("");
  const [slug, setSlug] = useState("");
  const [version, setVersion] = useState("");
  const [description, setDescription] = useState("");
  const createBuild = useCreateBuild();

  function handleOpenChange(next: boolean) {
    setOpen(next);
    if (!next) {
      setName("");
      setSlug("");
      setVersion("");
      setDescription("");
    }
  }

  function handleSubmit(event: React.SyntheticEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!name.trim() || !slug.trim()) {
      return;
    }
    createBuild.mutate(
      {
        name: name.trim(),
        slug: slug.trim(),
        version: version.trim() || null,
        description: description.trim() || null,
      },
      { onSuccess: () => handleOpenChange(false) },
    );
  }

  return (
    <Dialog open={open} onOpenChange={handleOpenChange}>
      <Button onClick={() => setOpen(true)}>
        <Plus className="size-4" />
        New build
      </Button>
      <DialogContent>
        <form onSubmit={handleSubmit}>
          <DialogHeader>
            <DialogTitle>Create build</DialogTitle>
            <DialogDescription>
              A build is a versioned set of overlay files the launcher syncs.
            </DialogDescription>
          </DialogHeader>
          <div className="mt-4 grid grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <Label htmlFor="build-name">Name</Label>
              <Input
                id="build-name"
                value={name}
                autoFocus
                placeholder="ATM9 Overlay"
                onChange={(event) => setName(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="build-slug">Slug</Label>
              <Input
                id="build-slug"
                value={slug}
                placeholder="atm9-overlay"
                onChange={(event) => setSlug(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="build-version">Version</Label>
              <Input
                id="build-version"
                value={version}
                placeholder="1.0.0"
                onChange={(event) => setVersion(event.target.value)}
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor="build-desc">Description</Label>
              <Input
                id="build-desc"
                value={description}
                onChange={(event) => setDescription(event.target.value)}
              />
            </div>
          </div>
          <DialogFooter className="mt-6">
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
              disabled={createBuild.isPending || !name.trim() || !slug.trim()}
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

function UploadArchiveCard({ slug }: { slug: string }) {
  const [file, setFile] = useState<File | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const uploadArchive = useUploadArchive();

  function submit() {
    if (!file) {
      return;
    }
    uploadArchive.mutate(
      { slug, file },
      {
        onSuccess: () => {
          setFile(null);
          if (inputRef.current) {
            inputRef.current.value = "";
          }
        },
      },
    );
  }

  return (
    <div className="space-y-3 rounded-md border border-edge-md bg-surface-1 p-4">
      <div className="flex items-center gap-2 text-body-med text-text-hi">
        <FileArchive className="size-4" />
        Upload ZIP archive
      </div>
      <p className="text-caption text-text-mute">
        The archive is extracted into the build, artifacts are hashed, and the
        manifest is regenerated.
      </p>
      <div className="flex flex-wrap items-center gap-2">
        <Input
          ref={inputRef}
          type="file"
          accept=".zip"
          disabled={uploadArchive.isPending}
          onChange={(event) => setFile(event.target.files?.[0] ?? null)}
          className="max-w-xs"
        />
        <Button
          onClick={submit}
          disabled={!file || uploadArchive.isPending}
        >
          {uploadArchive.isPending ? (
            <Loader2 className="size-4 animate-spin" />
          ) : (
            <Upload className="size-4" />
          )}
          Upload
        </Button>
      </div>
      {uploadArchive.isPending ? (
        <div className="space-y-1">
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-3">
            <div className="h-full w-1/3 animate-pulse rounded-full bg-cta" />
          </div>
          <p className="text-caption text-text-mute">
            Uploading and processing “{file?.name}”…
          </p>
        </div>
      ) : null}
    </div>
  );
}

function ValidationResultBlock({ result }: { result: ValidateResult }) {
  const clean = result.missing.length === 0 && result.orphaned.length === 0;
  return (
    <div className="space-y-2 rounded-md border border-edge-md bg-surface-1 p-3 text-caption">
      {clean ? (
        <p className="flex items-center gap-2 text-text">
          <CheckCircle2 className="size-4" />
          All artifact rows match files on disk.
        </p>
      ) : (
        <div className="space-y-2 text-text">
          {result.missing.length > 0 ? (
            <div>
              <p className="text-body-med text-text-hi">
                Missing files ({result.missing.length})
              </p>
              <ul className="mt-1 list-disc space-y-0.5 pl-5 text-text-mute">
                {result.missing.map((entry) => (
                  <li key={entry.id}>{entry.relativePath}</li>
                ))}
              </ul>
            </div>
          ) : null}
          {result.orphaned.length > 0 ? (
            <div>
              <p className="text-body-med text-text-hi">
                Orphaned files ({result.orphaned.length})
              </p>
              <ul className="mt-1 list-disc space-y-0.5 pl-5 text-text-mute">
                {result.orphaned.map((entry) => (
                  <li key={entry.relativePath}>{entry.relativePath}</li>
                ))}
              </ul>
            </div>
          ) : null}
        </div>
      )}
    </div>
  );
}

function ArtifactRow({
  slug,
  artifact,
}: {
  slug: string;
  artifact: BundleArtifact;
}) {
  const [confirmOpen, setConfirmOpen] = useState(false);
  const deleteFile = useDeleteFile();
  const Icon = artifact.isDir ? Folder : FileText;

  return (
    <TableRow>
      <TableCell>
        <span className="flex items-center gap-2 text-text">
          <Icon className="size-4 text-text-mute" />
          {artifact.relativePath}
        </span>
      </TableCell>
      <TableCell className="text-text-mute">
        {artifact.isDir ? "—" : formatBytes(artifact.size)}
      </TableCell>
      <TableCell>
        {artifact.downloadOnce ? (
          <Badge variant="secondary">Download once</Badge>
        ) : (
          <span className="text-text-faint">—</span>
        )}
      </TableCell>
      <TableCell className="text-right">
        <Button
          variant="ghost"
          size="icon"
          aria-label={`Delete ${artifact.relativePath}`}
          onClick={() => setConfirmOpen(true)}
        >
          <Trash2 className="size-4" />
        </Button>
        <ConfirmDialog
          open={confirmOpen}
          onOpenChange={setConfirmOpen}
          title={`Delete “${artifact.name}”?`}
          description={
            artifact.isDir
              ? "Deletes the folder and all its contents from the build."
              : "Removes the file from the build and disk."
          }
          confirmLabel="Delete"
          destructive
          pending={deleteFile.isPending}
          onConfirm={() =>
            deleteFile.mutate(
              { slug, entryId: artifact.id },
              { onSuccess: () => setConfirmOpen(false) },
            )
          }
        />
      </TableCell>
    </TableRow>
  );
}

function BuildDetailDialog({
  slug,
  open,
  onOpenChange,
}: {
  slug: string | null;
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const build = useBuild(slug ?? undefined);
  const regenerate = useRegenerateManifest();
  const validate = useValidateBuild();
  const [validation, setValidation] = useState<ValidateResult | null>(null);

  function runValidate() {
    if (!slug) {
      return;
    }
    validate.mutate(slug, { onSuccess: (result) => setValidation(result) });
  }

  const data = build.data;

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next) {
          setValidation(null);
        }
        onOpenChange(next);
      }}
    >
      <DialogContent className="max-h-[85vh] gap-0 overflow-y-auto sm:max-w-3xl">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2">
            <Boxes className="size-4" />
            {data?.name ?? slug}
          </DialogTitle>
          <DialogDescription>
            {data ? (
              <span className="flex flex-wrap items-center gap-x-3 gap-y-1">
                <span>{data.slug}</span>
                <span>{data.filesCount} files</span>
                <span>{formatBytes(data.totalSize)}</span>
                {data.version ? <span>v{data.version}</span> : null}
              </span>
            ) : null}
          </DialogDescription>
        </DialogHeader>

        {build.isLoading || !data ? (
          <div className="mt-4">
            <TableSkeleton rows={4} columns={3} />
          </div>
        ) : (
          <div className="mt-4 space-y-5">
            <div className="flex flex-wrap items-center gap-2">
              <StatusBadge bundle={data} />
              <Button
                variant="outline"
                size="sm"
                disabled={regenerate.isPending}
                onClick={() => regenerate.mutate(data.slug)}
              >
                {regenerate.isPending ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <RefreshCw className="size-4" />
                )}
                Regenerate
              </Button>
              <Button
                variant="outline"
                size="sm"
                disabled={validate.isPending}
                onClick={runValidate}
              >
                {validate.isPending ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : (
                  <ShieldCheck className="size-4" />
                )}
                Validate
              </Button>
              <Button variant="outline" size="sm" asChild>
                <a
                  href={manifestUrl(data.slug)}
                  target="_blank"
                  rel="noreferrer"
                >
                  <ExternalLink className="size-4" />
                  Manifest
                </a>
              </Button>
            </div>

            {data.processingError ? (
              <div className="flex items-start gap-2 rounded-md border border-edge-md bg-surface-1 p-3 text-caption text-text">
                <AlertTriangle className="mt-0.5 size-4 shrink-0" />
                <span>{data.processingError}</span>
              </div>
            ) : null}

            {validation ? <ValidationResultBlock result={validation} /> : null}

            <UploadArchiveCard slug={data.slug} />

            <Separator />

            <div>
              <p className="mb-2 text-body-med text-text-hi">
                Artifacts ({data.artifacts.length})
              </p>
              {data.artifacts.length === 0 ? (
                <EmptyState
                  icon={FileArchive}
                  title="No artifacts yet"
                  description="Upload a ZIP archive to populate this build."
                />
              ) : (
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead>Path</TableHead>
                      <TableHead>Size</TableHead>
                      <TableHead>Flags</TableHead>
                      <TableHead className="w-12" />
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {data.artifacts.map((artifact) => (
                      <ArtifactRow
                        key={artifact.id}
                        slug={data.slug}
                        artifact={artifact}
                      />
                    ))}
                  </TableBody>
                </Table>
              )}
            </div>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

function DiskSpaceCard() {
  const disk = useDiskSpace();
  const data = disk.data;
  const used =
    data?.free != null && data?.total != null ? data.total - data.free : null;
  const pct =
    used != null && data?.total ? Math.round((used / data.total) * 100) : null;

  return (
    <Card>
      <CardHeader>
        <CardDescription className="flex items-center gap-2">
          <HardDrive className="size-4" />
          Storage volume
        </CardDescription>
        <CardTitle className="text-h2 tabular-nums">
          {data?.free != null ? `${formatBytes(data.free)} free` : "Unknown"}
        </CardTitle>
      </CardHeader>
      {pct != null ? (
        <CardContent>
          <div className="h-1.5 w-full overflow-hidden rounded-full bg-surface-3">
            <div
              className="h-full rounded-full bg-text-mute"
              style={{ width: `${pct}%` }}
            />
          </div>
          <p className="mt-1 text-caption text-text-mute">
            {pct}% used of {data?.total != null ? formatBytes(data.total) : "—"}
          </p>
        </CardContent>
      ) : null}
    </Card>
  );
}

export function BundlesPage() {
  const builds = useBuilds();
  const deleteBuild = useDeleteBuild();
  const [detailSlug, setDetailSlug] = useState<string | null>(null);
  const [detailOpen, setDetailOpen] = useState(false);
  const [confirm, setConfirm] = useState<Bundle | null>(null);
  const rows = builds.data ?? [];

  function openDetail(slug: string) {
    setDetailSlug(slug);
    setDetailOpen(true);
  }

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Bundles"
        description="Build registry: artifacts, manifests, and uploads."
        actions={<CreateBuildDialog />}
      />

      <div className="grid grid-cols-1 gap-4 sm:grid-cols-3">
        <DiskSpaceCard />
      </div>

      <Card>
        <CardContent>
          {builds.isLoading ? (
            <TableSkeleton rows={5} columns={5} />
          ) : builds.isError ? (
            <EmptyState
              icon={Boxes}
              title="Could not load builds"
              action={
                <Button variant="outline" onClick={() => builds.refetch()}>
                  Retry
                </Button>
              }
            />
          ) : rows.length === 0 ? (
            <EmptyState
              icon={Boxes}
              title="No builds yet"
              description="Create a build, then upload a ZIP archive of overlay files."
            />
          ) : (
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Name</TableHead>
                  <TableHead>Status</TableHead>
                  <TableHead className="text-right">Files</TableHead>
                  <TableHead className="text-right">Size</TableHead>
                  <TableHead>Updated</TableHead>
                  <TableHead className="w-px text-right">Actions</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {rows.map((build) => (
                  <TableRow
                    key={build.id}
                    className="cursor-pointer"
                    onClick={() => openDetail(build.slug)}
                  >
                    <TableCell>
                      <div className="font-medium text-text-hi">
                        {build.name}
                      </div>
                      <div className="text-caption text-text-mute">
                        {build.slug}
                      </div>
                    </TableCell>
                    <TableCell>
                      <StatusBadge bundle={build} />
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-text-mute">
                      {build.filesCount}
                    </TableCell>
                    <TableCell className="text-right tabular-nums text-text-mute">
                      {formatBytes(build.totalSize)}
                    </TableCell>
                    <TableCell className="text-text-mute">
                      {formatDateTime(build.updatedAt)}
                    </TableCell>
                    <TableCell
                      onClick={(event) => event.stopPropagation()}
                    >
                      <div className="flex items-center justify-end gap-1">
                        <Button
                          variant="ghost"
                          size="sm"
                          onClick={() => openDetail(build.slug)}
                        >
                          Open
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          aria-label={`Delete ${build.name}`}
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
          )}
        </CardContent>
      </Card>

      <BuildDetailDialog
        slug={detailSlug}
        open={detailOpen}
        onOpenChange={setDetailOpen}
      />
      <ConfirmDialog
        open={confirm !== null}
        onOpenChange={(next) => !next && setConfirm(null)}
        title={`Delete build “${confirm?.name}”?`}
        description="Deletes the build, all artifact rows, and every file on disk. This cannot be undone."
        confirmLabel="Delete build"
        destructive
        pending={deleteBuild.isPending}
        onConfirm={() => {
          if (!confirm) {
            return;
          }
          deleteBuild.mutate(confirm.slug, {
            onSuccess: () => setConfirm(null),
          });
        }}
      />
    </div>
  );
}
