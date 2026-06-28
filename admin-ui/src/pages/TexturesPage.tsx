import {
  AlertCircle,
  ChevronLeft,
  ChevronRight,
  ImageOff,
  Search,
  Shirt,
  Trash2,
  Wand2,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";

import { ConfirmDialog } from "@/components/shared/ConfirmDialog";
import { PageHeader } from "@/components/shared/PageHeader";
import { SectionTabs } from "@/components/shared/SectionTabs";
import {
  TableSkeletonRows,
  TableStateRow,
} from "@/components/shared/TableStates";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import {
  type TextureKind,
  type TextureRow,
  useDeleteTexture,
  useOrphans,
  usePurgeMissing,
  useTextures,
} from "@/features/textures/api";
import { errorMessage } from "@/shared/api/toast";
import { formatDate, shortUuid } from "@/shared/lib/format";
import { useDebounced } from "@/shared/lib/useDebounced";

function formatBytes(bytes: number): string {
  if (!bytes) {
    return "—";
  }
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  return `${(bytes / 1024).toFixed(1)} KB`;
}

// The raw texture PNG, served public at `/textures/{uuid}/{skin|cape}`. The URL
// is revision-stable, so bust the browser cache with the row's `updatedAt`.
function TexturePreview({ row, kind }: { row: TextureRow; kind: TextureKind }) {
  const [failed, setFailed] = useState(false);
  const slug = kind === "skins" ? "skin" : "cape";
  const src = `/textures/${row.profileUuid}/${slug}?v=${encodeURIComponent(row.updatedAt)}`;

  if (failed) {
    return (
      <div className="flex size-10 items-center justify-center rounded border border-edge bg-surface-1 text-text-faint">
        <ImageOff className="size-4" />
      </div>
    );
  }
  return (
    <img
      src={src}
      alt={`${row.username}'s ${slug}`}
      width={40}
      height={40}
      onError={() => setFailed(true)}
      className="size-10 rounded border border-edge bg-surface-1 object-contain p-0.5"
      style={{ imageRendering: "pixelated" }}
    />
  );
}

const TABS = [
  { value: "skins" as const, label: "Skins", icon: Shirt },
  { value: "capes" as const, label: "Capes" },
];

export function TexturesPage() {
  const [kind, setKind] = useState<TextureKind>("skins");
  const [search, setSearch] = useState("");
  const [page, setPage] = useState(1);
  const [deleteTarget, setDeleteTarget] = useState<TextureRow | null>(null);
  const [orphansEnabled, setOrphansEnabled] = useState(false);
  const [purgeOpen, setPurgeOpen] = useState(false);

  const debouncedSearch = useDebounced(search.trim());

  // Reset to the first page whenever the kind or query changes.
  useEffect(() => {
    setPage(1);
  }, [debouncedSearch, kind]);

  // Switching kind also clears the (now-stale) search box.
  useEffect(() => {
    setSearch("");
  }, [kind]);

  const query = useMemo(
    () => ({ q: debouncedSearch || undefined, page }),
    [debouncedSearch, page],
  );
  const { data, isLoading, isError, error, isFetching } = useTextures(
    kind,
    query,
  );
  const deleteTexture = useDeleteTexture(kind);
  const orphans = useOrphans(orphansEnabled);
  const purgeMissing = usePurgeMissing();

  const rows = data?.data ?? [];
  const meta = data?.meta;
  const pageCount = meta?.pageCount ?? 0;
  const total = meta?.total ?? 0;
  const showEmpty = !isLoading && !isError && rows.length === 0;
  const showVariant = kind === "skins";
  const columns = showVariant ? 7 : 6;

  const orphanCount = orphans.data
    ? orphans.data.skins.length + orphans.data.capes.length
    : null;

  const confirmDelete = () => {
    if (!deleteTarget) {
      return;
    }
    deleteTexture.mutate(deleteTarget.userId, {
      onSuccess: () => setDeleteTarget(null),
    });
  };

  return (
    <div className="flex flex-col gap-6">
      <PageHeader
        title="Textures"
        description="The skin & cape registry. Browse, preview, and remove player textures, or clean up rows whose file is missing."
        actions={
          <>
            {orphanCount !== null && (
              <Badge variant={orphanCount > 0 ? "destructive" : "outline"}>
                {orphanCount} orphan{orphanCount === 1 ? "" : "s"}
              </Badge>
            )}
            <Button
              variant="outline"
              disabled={orphans.isFetching}
              onClick={() => {
                if (orphansEnabled) {
                  void orphans.refetch();
                } else {
                  setOrphansEnabled(true);
                }
              }}
            >
              <Wand2 className="size-4" />
              Scan orphans
            </Button>
            <Button
              variant="destructive"
              disabled={purgeMissing.isPending || orphanCount === 0}
              onClick={() => setPurgeOpen(true)}
            >
              <Trash2 className="size-4" />
              Purge missing
            </Button>
          </>
        }
      />

      <div className="flex flex-col gap-4 sm:flex-row sm:items-center sm:justify-between">
        <SectionTabs tabs={TABS} value={kind} onChange={setKind} />
        <div className="relative max-w-sm flex-1">
          <Search className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-text-faint" />
          <Input
            value={search}
            onChange={(event) => setSearch(event.target.value)}
            placeholder="Search by username or profile UUID"
            aria-label="Search textures"
            className="pl-9"
          />
        </div>
      </div>

      <div className="rounded-lg border border-edge bg-card">
        <Table>
          <TableHeader>
            <TableRow className="hover:bg-transparent">
              <TableHead className="w-16">Preview</TableHead>
              <TableHead>Username</TableHead>
              <TableHead>Profile UUID</TableHead>
              {showVariant && <TableHead>Variant</TableHead>}
              <TableHead>Size</TableHead>
              <TableHead>Updated</TableHead>
              <TableHead className="text-right">Actions</TableHead>
            </TableRow>
          </TableHeader>
          <TableBody>
            {isLoading && <TableSkeletonRows columns={columns} />}

            {isError && (
              <TableStateRow
                columns={columns}
                icon={AlertCircle}
                title="Could not load textures"
                description={errorMessage(error, "Please try again.")}
              />
            )}

            {showEmpty && (
              <TableStateRow
                columns={columns}
                icon={ImageOff}
                title={debouncedSearch ? "No matches" : "No textures yet"}
                description={
                  debouncedSearch
                    ? `Nothing matched "${debouncedSearch}".`
                    : `No ${kind} have been uploaded.`
                }
              />
            )}

            {!isLoading &&
              !isError &&
              rows.map((row) => (
                <TableRow key={row.userId}>
                  <TableCell>
                    <TexturePreview row={row} kind={kind} />
                  </TableCell>
                  <TableCell className="font-medium text-text-hi">
                    {row.username}
                  </TableCell>
                  <TableCell>
                    <code
                      className="font-mono text-caption text-text-mute"
                      title={row.profileUuid}
                    >
                      {shortUuid(row.profileUuid)}
                    </code>
                  </TableCell>
                  {showVariant && (
                    <TableCell>
                      <Badge variant="secondary">
                        {row.variant ?? "CLASSIC"}
                      </Badge>
                    </TableCell>
                  )}
                  <TableCell className="text-text-mute">
                    {formatBytes(row.fileSize)}
                  </TableCell>
                  <TableCell className="text-text-mute">
                    {formatDate(row.updatedAt)}
                  </TableCell>
                  <TableCell className="text-right">
                    <Button
                      variant="ghost"
                      size="icon"
                      aria-label={`Delete ${row.username}'s ${kind === "skins" ? "skin" : "cape"}`}
                      onClick={() => setDeleteTarget(row)}
                    >
                      <Trash2 className="size-4 text-destructive" />
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
          </TableBody>
        </Table>
      </div>

      <footer className="flex items-center justify-between gap-4">
        <p className="text-caption text-text-faint">
          {isLoading
            ? "Loading…"
            : total === 0
              ? `No ${kind}`
              : `${total.toLocaleString()} ${kind === "skins" ? (total === 1 ? "skin" : "skins") : total === 1 ? "cape" : "capes"}`}
          {isFetching && !isLoading && " · refreshing…"}
        </p>
        <div className="flex items-center gap-2">
          <span className="text-caption text-text-mute">
            Page {meta?.page ?? page} of {Math.max(pageCount, 1)}
          </span>
          <Button
            variant="outline"
            size="icon"
            aria-label="Previous page"
            disabled={page <= 1 || isLoading}
            onClick={() => setPage((value) => Math.max(1, value - 1))}
          >
            <ChevronLeft className="size-4" />
          </Button>
          <Button
            variant="outline"
            size="icon"
            aria-label="Next page"
            disabled={page >= pageCount || isLoading}
            onClick={() => setPage((value) => value + 1)}
          >
            <ChevronRight className="size-4" />
          </Button>
        </div>
      </footer>

      <ConfirmDialog
        open={deleteTarget !== null}
        onOpenChange={(open) => !open && setDeleteTarget(null)}
        title={`Delete ${kind === "skins" ? "skin" : "cape"}?`}
        description={
          deleteTarget
            ? `This removes ${deleteTarget.username}'s ${kind === "skins" ? "skin" : "cape"} from the registry and deletes the file. This cannot be undone.`
            : undefined
        }
        confirmLabel="Delete"
        destructive
        pending={deleteTexture.isPending}
        onConfirm={confirmDelete}
      />

      <ConfirmDialog
        open={purgeOpen}
        onOpenChange={(open) => {
          if (!open) {
            setPurgeOpen(false);
          }
        }}
        title="Purge missing textures?"
        description="Deletes every registry row whose backing file is gone from disk (across skins and capes). Run a scan first to see how many are affected."
        confirmLabel="Purge"
        destructive
        pending={purgeMissing.isPending}
        onConfirm={() =>
          purgeMissing.mutate(undefined, {
            onSuccess: () => {
              setPurgeOpen(false);
              if (orphansEnabled) {
                void orphans.refetch();
              }
            },
          })
        }
      />
    </div>
  );
}
