import {
  AlertTriangle,
  CheckCircle2,
  FolderOpen,
  HardDriveDownload,
  Loader2,
} from "lucide-react";
import type { DragEvent as ReactDragEvent } from "react";
import { useEffect, useMemo, useRef, useState } from "react";
import { toast } from "sonner";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { EmptyState } from "@/components/shared/EmptyState";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent } from "@/components/ui/card";
import { useUpdateClient } from "@/features/builds/api";
import {
  type FileMenuAction,
  FileContextMenu,
  type FileMenuTarget,
} from "@/features/builds/FileContextMenu";
import { FileBreadcrumbs } from "@/features/builds/FileBreadcrumbs";
import { FileGrid } from "@/features/builds/FileGrid";
import { FileList } from "@/features/builds/FileList";
import {
  buildTree,
  childrenOf,
  type FileTreeNode,
  joinPath,
  parentPath,
  type TreeEntry,
} from "@/features/builds/fileTree";
import type { MoveRequest } from "@/features/builds/dndHooks";
import { downloadFile } from "@/features/builds/download";
import { MoveDialog } from "@/features/builds/MoveDialog";
import { NewFolderDialog } from "@/features/builds/NewFolderDialog";
import { RenameDialog } from "@/features/builds/RenameDialog";
import { SelectionToolbar } from "@/features/builds/SelectionToolbar";
import {
  FileManagerToolbar,
  type ViewMode,
} from "@/features/builds/FileManagerToolbar";
import {
  useBuild,
  useBulkDelete,
  useDeleteFile,
  useMoveFiles,
  useRegenerateManifest,
  useRehashFile,
  useToggleDownloadOnce,
  useUploadArchive,
  useUploadFile,
  useValidateBuild,
} from "@/features/bundles/api";
import { cn } from "@/shared/lib/cn";
import { formatBytes, formatDateTime } from "@/shared/lib/format";
import type { Bundle, ClientAdmin } from "@/shared/types";

function StatusBadge({ status }: { status: string }) {
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
  return <Badge variant="secondary">Draft</Badge>;
}

function FooterStatus({ build }: { build: Bundle }) {
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-2 rounded-md border border-edge-md bg-surface-1 px-4 py-3 text-caption text-text-mute">
      <StatusBadge status={build.status} />
      <span>
        <span className="text-text-hi tabular-nums">{build.filesCount}</span>{" "}
        file{build.filesCount === 1 ? "" : "s"}
      </span>
      <span>
        <span className="text-text-hi tabular-nums">
          {formatBytes(build.totalSize)}
        </span>{" "}
        total
      </span>
      <span>
        Generated{" "}
        <span className="text-text-hi">
          {formatDateTime(build.lastGeneratedAt)}
        </span>
      </span>
    </div>
  );
}

/// The no-bundle heal CTA: re-saving the build with `bundleSlug: null` makes the
/// backend re-derive + re-provision the owned bundle (Layer C), after which the live
/// bundle read succeeds and the file manager loads.
function NoBundleState({ build }: { build: ClientAdmin }) {
  const updateClient = useUpdateClient();

  function heal() {
    updateClient.mutate({
      id: build.id,
      slug: build.slug,
      available: build.available,
      minecraftVersion: build.minecraftVersion,
      runtimeVersion: build.runtimeVersion,
      fabricVersion: build.fabricVersion,
      forgeVersion: build.forgeVersion,
      bundleSlug: null,
      locales: [
        {
          locale: "en",
          title: build.title,
          shortDescription: build.shortDescription || null,
          description: build.description || null,
        },
      ],
    });
  }

  return (
    <Card>
      <CardContent className="py-12">
        <EmptyState
          icon={HardDriveDownload}
          title="No file storage yet"
          description="This build has no bundle to store files in. Set one up to start uploading mods, configs, and resource packs."
          action={
            <Button onClick={heal} disabled={updateClient.isPending}>
              {updateClient.isPending ? (
                <Loader2 className="size-4 animate-spin" />
              ) : null}
              Set up file storage
            </Button>
          }
        />
      </CardContent>
    </Card>
  );
}

export function BuildFilesTab({ build }: { build: ClientAdmin }) {
  const bundleSlug = build.bundle?.slug ?? null;
  const query = useBuild(bundleSlug ?? undefined);

  const [currentPath, setCurrentPath] = useState("");
  const [selectedKeys, setSelectedKeys] = useState<Set<string>>(new Set());
  const [viewMode, setViewMode] = useState<ViewMode>("list");
  const [dropActive, setDropActive] = useState(false);
  const lastClickedRef = useRef<string | null>(null);

  const [newFolderOpen, setNewFolderOpen] = useState(false);
  const [bulkConfirmOpen, setBulkConfirmOpen] = useState(false);
  const [moveOpen, setMoveOpen] = useState(false);
  const [moveSources, setMoveSources] = useState<string[]>([]);
  const [renameEntry, setRenameEntry] = useState<TreeEntry | null>(null);
  const [deleteEntry, setDeleteEntry] = useState<TreeEntry | null>(null);
  const [menuTarget, setMenuTarget] = useState<FileMenuTarget | null>(null);
  const [menuOpen, setMenuOpen] = useState(false);

  const uploadFile = useUploadFile();
  const uploadArchive = useUploadArchive();
  const regenerate = useRegenerateManifest();
  const validate = useValidateBuild();
  const bulkDelete = useBulkDelete();
  const moveFiles = useMoveFiles();
  const deleteFile = useDeleteFile();
  const toggleOnce = useToggleDownloadOnce();
  const rehash = useRehashFile();

  const data = query.data;

  const roots = useMemo(
    () => (data ? buildTree(data.artifacts) : []),
    [data],
  );
  const nodesById = useMemo(() => {
    const map = new Map<string, FileTreeNode>();
    const walk = (nodes: FileTreeNode[]) => {
      for (const node of nodes) {
        map.set(node.id, node);
        walk(node.children);
      }
    };
    walk(roots);
    return map;
  }, [roots]);

  const children = useMemo(
    () => (data ? childrenOf(data.artifacts, currentPath) : []),
    [data, currentPath],
  );

  // Self-heal the phantom folder: after a delete / move / bulk-delete the current
  // folder may no longer exist among the artifacts. Clamp `currentPath` to the
  // nearest existing ancestor (or root) whenever the data changes so the breadcrumb
  // never points nowhere.
  useEffect(() => {
    if (!data || currentPath === "") {
      return;
    }
    const exists = (path: string) =>
      path === "" ||
      data.artifacts.some(
        (a) =>
          a.relativePath === path || a.relativePath.startsWith(`${path}/`),
      );
    if (exists(currentPath)) {
      return;
    }
    let path = currentPath;
    while (path !== "" && !exists(path)) {
      path = parentPath(path);
    }
    setCurrentPath(path);
  }, [data, currentPath]);

  const slug = data?.slug ?? "";

  function navigate(path: string) {
    setCurrentPath(path);
    setSelectedKeys(new Set());
  }

  function uploadFiles(files: FileList | File[], dir = currentPath) {
    for (const file of Array.from(files)) {
      uploadFile.mutate({ slug, file, targetPath: joinPath(dir, file.name) });
    }
  }

  function onMove(req: MoveRequest) {
    moveFiles.mutate(
      { slug, ids: req.ids, targetDir: req.targetDir },
      { onSuccess: () => setSelectedKeys(new Set()) },
    );
  }

  // Selection is artifact-only: folders carry no id, so they are never selectable
  // via a checkbox. Then `selectedCount` always equals `selectedArtifactIds.length`
  // and the bulk-delete dialog count never lies.
  const selectableChildren = useMemo(
    () => children.filter((c) => c.artifact),
    [children],
  );

  function toggleOne(relativePath: string, shiftKey: boolean) {
    setSelectedKeys((prev) => {
      const next = new Set(prev);
      if (shiftKey && lastClickedRef.current) {
        const order = selectableChildren.map((c) => c.relativePath);
        const a = order.indexOf(lastClickedRef.current);
        const b = order.indexOf(relativePath);
        if (a !== -1 && b !== -1) {
          const [lo, hi] = a < b ? [a, b] : [b, a];
          for (let i = lo; i <= hi; i++) {
            next.add(order[i]);
          }
          lastClickedRef.current = relativePath;
          return next;
        }
      }
      if (next.has(relativePath)) {
        next.delete(relativePath);
      } else {
        next.add(relativePath);
      }
      lastClickedRef.current = relativePath;
      return next;
    });
  }

  function toggleAll(checked: boolean) {
    setSelectedKeys(
      checked
        ? new Set(selectableChildren.map((c) => c.relativePath))
        : new Set(),
    );
  }

  const allSelected =
    selectableChildren.length > 0 &&
    selectableChildren.every((c) => selectedKeys.has(c.relativePath));
  const someSelected =
    !allSelected && selectableChildren.some((c) => selectedKeys.has(c.relativePath));

  // The name button: folders navigate, files select. (The kebab "Open" still
  // downloads files via `openEntry`.)
  function onNameAction(relativePath: string) {
    const entry = children.find((e) => e.relativePath === relativePath);
    if (!entry) {
      return;
    }
    if (entry.isDir) {
      navigate(entry.relativePath);
    } else {
      toggleOne(relativePath, false);
    }
  }

  // The kebab "Open" action: folders navigate, files download.
  function openEntry(relativePath: string) {
    const entry = children.find((e) => e.relativePath === relativePath);
    if (!entry) {
      return;
    }
    if (entry.isDir) {
      navigate(entry.relativePath);
    } else {
      toast.message(`Downloading ${entry.name}…`);
      downloadFile(slug, entry.relativePath);
    }
  }

  function onToggleDownloadOnceEntry(entry: TreeEntry) {
    if (entry.artifact) {
      toggleOnce.mutate({
        slug,
        entryId: entry.artifact.id,
        downloadOnce: !entry.artifact.downloadOnce,
      });
    }
  }

  function onBodyDrop(e: ReactDragEvent) {
    e.preventDefault();
    setDropActive(false);
    if (e.dataTransfer.files.length > 0) {
      uploadFiles(e.dataTransfer.files);
    }
  }

  function openMenu(entry: TreeEntry, position: { x: number; y: number }) {
    setMenuTarget({ entry, position });
    setMenuOpen(true);
  }

  function onMenuAction(action: FileMenuAction, entry: TreeEntry) {
    switch (action) {
      case "open":
        openEntry(entry.relativePath);
        break;
      case "download":
        downloadFile(slug, entry.relativePath);
        break;
      case "rename":
        setRenameEntry(entry);
        break;
      case "move":
        setMoveSources([entry.relativePath]);
        setMoveOpen(true);
        break;
      case "toggle-once":
        if (entry.artifact) {
          toggleOnce.mutate({
            slug,
            entryId: entry.artifact.id,
            downloadOnce: !entry.artifact.downloadOnce,
          });
        }
        break;
      case "rehash":
        if (entry.artifact) {
          rehash.mutate({ slug, entryId: entry.artifact.id });
        }
        break;
      case "delete":
        setDeleteEntry(entry);
        break;
    }
  }

  // The selected artifact ids drive bulk delete / move; implied folders carry no id.
  const selectedEntries = [...selectedKeys]
    .map((key) => nodesById.get(String(key)))
    .filter((node): node is FileTreeNode => Boolean(node));
  const selectedArtifactIds = selectedEntries
    .map((node) => node.artifact?.id)
    .filter((id): id is string => Boolean(id));
  const selectedCount = selectedKeys.size;

  function runValidate() {
    validate.mutate(slug, {
      onSuccess: (result) => {
        if (result.missing.length === 0 && result.orphaned.length === 0) {
          toast.success("All artifact rows match files on disk");
        } else {
          toast.warning(
            `${result.missing.length} missing, ${result.orphaned.length} orphaned`,
          );
        }
      },
    });
  }

  function downloadSelection() {
    let skipped = 0;
    for (const node of selectedEntries) {
      if (node.isDir) {
        skipped += 1;
        continue;
      }
      downloadFile(slug, node.relativePath);
    }
    if (skipped > 0) {
      toast.info(
        `Skipped ${skipped} folder${skipped === 1 ? "" : "s"} — no folder download`,
      );
    }
  }

  // --- Branches (order matters: error BEFORE loading; Layer A→null, B→error) ---

  if (bundleSlug === null) {
    return <NoBundleState build={build} />;
  }

  if (query.isError) {
    const notFound =
      query.error instanceof Error && /404|not\s*found/i.test(query.error.message);
    return (
      <Card>
        <CardContent className="py-12">
          <EmptyState
            icon={AlertTriangle}
            title="Couldn’t load files"
            description={
              notFound
                ? "This build points to a bundle that no longer exists."
                : "The file storage for this build could not be loaded."
            }
            action={
              <Button
                variant="outline"
                onClick={() => query.refetch()}
                disabled={query.isFetching}
              >
                {query.isFetching ? (
                  <Loader2 className="size-4 animate-spin" />
                ) : null}
                Retry
              </Button>
            }
          />
        </CardContent>
      </Card>
    );
  }

  if (query.isLoading || !data) {
    return (
      <Card>
        <CardContent>
          <div className="grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3">
            {Array.from({ length: 8 }).map((_, i) => (
              <div
                // biome-ignore lint/suspicious/noArrayIndexKey: static skeletons
                key={i}
                className="h-24 animate-pulse rounded-lg border border-edge-md bg-surface-1"
              />
            ))}
          </div>
        </CardContent>
      </Card>
    );
  }

  return (
    <Card>
      <CardContent className="space-y-4">
        <FileManagerToolbar
          viewMode={viewMode}
          onViewModeChange={setViewMode}
          onNewFolder={() => setNewFolderOpen(true)}
          onUploadFiles={(files) => uploadFiles(files)}
          onUploadArchive={(file) => uploadArchive.mutate({ slug, file })}
          onRegenerate={() => regenerate.mutate(slug)}
          onValidate={runValidate}
          uploadPending={uploadFile.isPending}
          archivePending={uploadArchive.isPending}
          regeneratePending={regenerate.isPending}
          validatePending={validate.isPending}
        />

        <FileBreadcrumbs
          currentPath={currentPath}
          onNavigate={navigate}
          onMoveToRoot={(ids) => onMove({ ids, targetDir: "" })}
          onUploadToRoot={(files) => uploadFiles(files, "")}
        />

        {selectedCount > 0 ? (
          <SelectionToolbar
            count={selectedCount}
            onClear={() => setSelectedKeys(new Set())}
            onMove={() => {
              setMoveSources(selectedEntries.map((n) => n.relativePath));
              setMoveOpen(true);
            }}
            onDownload={downloadSelection}
            onDelete={() => setBulkConfirmOpen(true)}
          />
        ) : null}

        {/* Upload drop-wrapper around the table/grid body only — the FileBreadcrumbs
            Root DropZone owns root drops, so the two regions are disjoint. */}
        <div
          className={cn(
            "rounded-lg outline-none transition-shadow",
            dropActive && "ring-2 ring-ring ring-inset",
          )}
          onDragOver={(e) => {
            if (Array.from(e.dataTransfer.types).includes("Files")) {
              e.preventDefault();
              setDropActive(true);
            }
          }}
          onDragLeave={(e) => {
            if (!e.currentTarget.contains(e.relatedTarget as Node)) {
              setDropActive(false);
            }
          }}
          onDrop={onBodyDrop}
        >
          {children.length === 0 ? (
            <EmptyState
              icon={FolderOpen}
              title="This folder is empty"
              description="Upload files or a ZIP, drag files here, or move entries in from another folder."
            />
          ) : viewMode === "grid" ? (
            <FileGrid
              entries={children}
              selectedKeys={selectedKeys}
              onToggle={toggleOne}
              onToggleAll={toggleAll}
              allSelected={allSelected}
              someSelected={someSelected}
              onAction={onNameAction}
              onOpenMenu={openMenu}
              onToggleDownloadOnce={onToggleDownloadOnceEntry}
            />
          ) : (
            <FileList
              entries={children}
              selectedKeys={selectedKeys}
              onToggle={toggleOne}
              onToggleAll={toggleAll}
              allSelected={allSelected}
              someSelected={someSelected}
              onAction={onNameAction}
              onOpenMenu={openMenu}
              onToggleDownloadOnce={onToggleDownloadOnceEntry}
            />
          )}
        </div>

        {data.processingError ? (
          <div className="flex items-start gap-2 rounded-md border border-edge-md bg-surface-1 p-3 text-caption text-text">
            <AlertTriangle className="mt-0.5 size-4 shrink-0" />
            <span>{data.processingError}</span>
          </div>
        ) : null}

        <FooterStatus build={data} />
      </CardContent>

      <FileContextMenu
        open={menuOpen}
        onOpenChange={setMenuOpen}
        target={menuTarget}
        onAction={onMenuAction}
      />

      <NewFolderDialog
        slug={slug}
        currentPath={currentPath}
        open={newFolderOpen}
        onOpenChange={setNewFolderOpen}
      />

      <MoveDialog
        open={moveOpen}
        onOpenChange={setMoveOpen}
        roots={roots}
        sourcePaths={moveSources}
        pending={moveFiles.isPending}
        onMove={(targetDir) => {
          const ids = moveSources
            .map((path) => nodesById.get(path)?.artifact?.id)
            .filter((id): id is string => Boolean(id));
          if (ids.length === 0) {
            return;
          }
          moveFiles.mutate(
            { slug, ids, targetDir },
            {
              onSuccess: () => {
                setMoveOpen(false);
                setSelectedKeys(new Set());
              },
            },
          );
        }}
      />

      {renameEntry ? (
        <RenameDialog
          slug={slug}
          entry={renameEntry}
          open={renameEntry !== null}
          onOpenChange={(open) => !open && setRenameEntry(null)}
        />
      ) : null}

      {deleteEntry?.artifact ? (
        <ConfirmDialog
          open={deleteEntry !== null}
          onOpenChange={(open) => !open && setDeleteEntry(null)}
          title={`Delete “${deleteEntry.name}”?`}
          description={
            deleteEntry.isDir
              ? "Deletes the folder and all its contents from the build."
              : "Removes the file from the build and disk."
          }
          confirmLabel="Delete"
          destructive
          pending={deleteFile.isPending}
          onConfirm={() => {
            const artifact = deleteEntry.artifact;
            if (!artifact) {
              return;
            }
            deleteFile.mutate(
              { slug, entryId: artifact.id },
              { onSuccess: () => setDeleteEntry(null) },
            );
          }}
        />
      ) : null}

      <ConfirmDialog
        open={bulkConfirmOpen}
        onOpenChange={setBulkConfirmOpen}
        title={`Delete ${selectedArtifactIds.length} ${selectedArtifactIds.length === 1 ? "entry" : "entries"}?`}
        description="Removes the selected entries from the build and disk. This cannot be undone."
        confirmLabel="Delete selected"
        destructive
        pending={bulkDelete.isPending}
        onConfirm={() =>
          bulkDelete.mutate(
            { slug, ids: selectedArtifactIds },
            {
              onSuccess: () => {
                setBulkConfirmOpen(false);
                setSelectedKeys(new Set());
              },
            },
          )
        }
      />
    </Card>
  );
}
