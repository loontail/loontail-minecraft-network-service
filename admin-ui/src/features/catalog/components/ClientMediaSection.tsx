import { Loader2, RefreshCw, Trash2 } from "lucide-react";

import { FileUpload } from "@/components/shared/FileUpload";
import { Button } from "@/components/ui/button";
import {
  useClientMedia,
  useDeleteMedia,
  useUploadMedia,
} from "@/features/catalog/api";
import type { MediaRole } from "@/shared/types";

const IMAGE_HINT = "PNG, JPG, WebP or GIF · up to 32MB";

export const SINGULAR_SLOTS: { role: MediaRole; label: string }[] = [
  { role: "poster", label: "Poster" },
  { role: "background", label: "Background" },
  { role: "titleImage", label: "Title image" },
];

export function MediaSlotInput({
  clientId,
  role,
  label,
  current,
}: {
  clientId: string;
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
      upload.mutate({ clientId, role, file });
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
                deleteMedia.mutate({ clientId, mediaId: current.id })
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
  clientId,
  shots,
}: {
  clientId: string;
  shots: { id: string; url: string }[];
}) {
  const upload = useUploadMedia();
  const deleteMedia = useDeleteMedia();

  function addShots(files: File[]) {
    for (const file of files) {
      upload.mutate({ clientId, role: "screenshot", file });
    }
  }

  return (
    <div className="space-y-2">
      <div className="flex items-center justify-between">
        <span className="text-body-med text-text-hi">Screenshots</span>
        {upload.isPending ? (
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
                  deleteMedia.mutate({ clientId, mediaId: shot.id })
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
          disabled={upload.isPending}
          onFiles={addShots}
          label="Add screenshots"
          className="aspect-video"
        >
          {IMAGE_HINT}
        </FileUpload>
      </div>
    </div>
  );
}

export function ClientMediaSection({ clientId }: { clientId: string }) {
  const media = useClientMedia(clientId);
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
      ) : (
        <div className="space-y-5">
          <div className="grid gap-4 sm:grid-cols-3">
            {SINGULAR_SLOTS.map((slot) => (
              <MediaSlotInput
                key={slot.role}
                clientId={clientId}
                role={slot.role}
                label={slot.label}
                current={slotFor(slot.role)}
              />
            ))}
          </div>
          <ScreenshotsGallery clientId={clientId} shots={shots} />
        </div>
      )}
    </div>
  );
}
