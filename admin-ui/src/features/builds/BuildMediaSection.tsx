import { AlertTriangle, Loader2, RefreshCw, Trash2 } from "lucide-react";
import { useState } from "react";

import { EmptyState } from "@/components/shared/EmptyState";
import { FileUpload } from "@/components/shared/FileUpload";
import { Button } from "@/components/ui/button";
import {
  useBuildMedia,
  useDeleteMedia,
  useInvalidateBuildMedia,
  useUploadMedia,
} from "@/features/builds/api";
import {
  toastBatchUpload,
  uploadSequentially,
} from "@/shared/lib/batchUpload";
import type { MediaRole } from "@/shared/types";

const IMAGE_HINT = "PNG, JPG, WebP or GIF · up to 32MB";

export const SINGULAR_SLOTS: { role: MediaRole; label: string }[] = [
  { role: "poster", label: "Poster" },
  { role: "background", label: "Background" },
  { role: "titleImage", label: "Title image" },
];

export function MediaSlotInput({
  buildId,
  role,
  label,
  current,
}: {
  buildId: string;
  role: MediaRole;
  label: string;
  current: { id: string; url: string } | null;
}) {
  const upload = useUploadMedia();
  const deleteMedia = useDeleteMedia();
  const pending = upload.isPending || deleteMedia.isPending;

  function send(files: File[]) {
    const file = files[0];
    if (file) {
      upload.mutate({ buildId, role, file });
    }
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-body-med text-text-hi">{label}</span>
        {pending ? (
          <Loader2 className="size-4 animate-spin text-text-mute" />
        ) : null}
      </div>
      {current ? (
        <div className="group relative aspect-video overflow-hidden rounded-md border border-edge-md bg-surface-1">
          <img
            src={current.url}
            alt={label}
            className="size-full object-cover"
          />
          <div className="absolute inset-0 flex items-center justify-center gap-2 bg-overlay/70 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
            <ReplaceButton
              role={role}
              disabled={pending}
              onFiles={send}
            />
            <Button
              variant="outline"
              size="sm"
              disabled={pending}
              aria-label={`Remove ${label.toLowerCase()}`}
              onClick={() =>
                deleteMedia.mutate({ buildId, mediaId: current.id })
              }
            >
              <Trash2 className="size-4" />
              Remove
            </Button>
          </div>
        </div>
      ) : (
        <FileUpload
          accept="image/*"
          disabled={pending}
          onFiles={send}
          label="Drop image or click"
          className="aspect-video"
        >
          {IMAGE_HINT}
        </FileUpload>
      )}
    </div>
  );
}

function ReplaceButton({
  role,
  disabled,
  onFiles,
}: {
  role: MediaRole;
  disabled: boolean;
  onFiles: (files: File[]) => void;
}) {
  return (
    <Button asChild variant="outline" size="sm" disabled={disabled}>
      <label className={disabled ? "pointer-events-none" : "cursor-pointer"}>
        <RefreshCw className="size-4" />
        Replace
        <input
          type="file"
          accept="image/*"
          hidden
          disabled={disabled}
          aria-label={`Replace ${role}`}
          onChange={(event) => {
            const files = event.target.files;
            if (files && files.length > 0) {
              onFiles(Array.from(files));
            }
            event.target.value = "";
          }}
        />
      </label>
    </Button>
  );
}

export function ScreenshotsGallery({
  buildId,
  shots,
}: {
  buildId: string;
  shots: { id: string; url: string }[];
}) {
  const upload = useUploadMedia();
  const deleteMedia = useDeleteMedia();
  const invalidateMedia = useInvalidateBuildMedia();
  const [batchUploading, setBatchUploading] = useState(false);
  const uploading = upload.isPending || batchUploading;

  // Upload sequentially (see uploadSequentially) with `silent: true`, so the
  // per-call toast and invalidation are replaced by one summary toast and a single
  // invalidation here — the server-assigned `sortOrder` stays deterministic too.
  async function addShots(files: File[]) {
    if (files.length === 0) {
      return;
    }
    if (files.length === 1) {
      upload.mutate({ buildId, role: "screenshot", file: files[0] });
      return;
    }
    setBatchUploading(true);
    const outcome = await uploadSequentially(files, (file) =>
      upload.mutateAsync({
        buildId,
        role: "screenshot",
        file,
        silent: true,
      }),
    );
    setBatchUploading(false);
    invalidateMedia(buildId);
    toastBatchUpload(outcome, "screenshot");
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-body-med text-text-hi">Screenshots</span>
        {uploading ? (
          <Loader2 className="size-4 animate-spin text-text-mute" />
        ) : null}
      </div>
      <div className="grid grid-cols-2 gap-3 sm:grid-cols-3 lg:grid-cols-4">
        {shots.map((shot) => (
          <div
            key={shot.id}
            className="group relative aspect-video overflow-hidden rounded-md border border-edge-md bg-surface-1"
          >
            <img
              src={shot.url}
              alt="Screenshot"
              className="size-full object-cover"
            />
            <div className="absolute inset-0 flex items-center justify-center bg-overlay/70 opacity-0 transition-opacity group-hover:opacity-100 focus-within:opacity-100">
              <Button
                variant="outline"
                size="sm"
                aria-label="Remove screenshot"
                disabled={deleteMedia.isPending}
                onClick={() =>
                  deleteMedia.mutate({ buildId, mediaId: shot.id })
                }
              >
                <Trash2 className="size-4" />
                Remove
              </Button>
            </div>
          </div>
        ))}
        <FileUpload
          accept="image/*"
          multiple
          disabled={uploading}
          onFiles={(files) => void addShots(files)}
          label="Add screenshots"
          className="aspect-video"
        >
          {IMAGE_HINT}
        </FileUpload>
      </div>
    </div>
  );
}

export function BuildMediaSection({ buildId }: { buildId: string }) {
  const media = useBuildMedia(buildId);
  const rows = media.data ?? [];

  function slotFor(role: MediaRole): { id: string; url: string } | null {
    const row = rows.find((m) => m.role === role);
    return row ? { id: row.id, url: row.url } : null;
  }

  const shots = rows
    .filter((m) => m.role === "screenshot")
    .map((m) => ({ id: m.id, url: m.url }));

  return (
    <div className="space-y-4">
      {media.isLoading ? (
        <div className="flex items-center gap-2 text-caption text-text-mute">
          <Loader2 className="size-4 animate-spin" />
          Loading media…
        </div>
      ) : media.isError ? (
        <EmptyState
          icon={AlertTriangle}
          title="Couldn’t load media"
          description="The media for this build failed to load. Retry before re-uploading — existing images may still be attached."
          action={
            <Button
              variant="outline"
              onClick={() => media.refetch()}
              disabled={media.isFetching}
            >
              {media.isFetching ? (
                <Loader2 className="size-4 animate-spin" />
              ) : (
                <RefreshCw className="size-4" />
              )}
              Retry
            </Button>
          }
        />
      ) : (
        <div className="space-y-5">
          <div className="grid gap-4 sm:grid-cols-3">
            {SINGULAR_SLOTS.map((slot) => (
              <MediaSlotInput
                key={slot.role}
                buildId={buildId}
                role={slot.role}
                label={slot.label}
                current={slotFor(slot.role)}
              />
            ))}
          </div>
          <ScreenshotsGallery buildId={buildId} shots={shots} />
        </div>
      )}
    </div>
  );
}
