import { ChevronRight } from "lucide-react";
import {
  DropZone,
  isFileDropItem,
  isTextDropItem,
} from "react-aria-components";

import { DRAG_TYPE } from "@/features/builds/dndHooks";
import { breadcrumbs } from "@/features/builds/fileTree";
import { cn } from "@/shared/lib/cn";

interface DragPayload {
  ids: string[];
  paths: string[];
}

/// Google-Drive-style breadcrumb trail (Root / mods / config). Every crumb except
/// the last navigates on click. The Root crumb is also a drop target: dropping cards
/// on it moves them to the build root; dropping OS files uploads them to the root.
export function FileBreadcrumbs({
  currentPath,
  onNavigate,
  onMoveToRoot,
  onUploadToRoot,
}: {
  currentPath: string;
  onNavigate: (path: string) => void;
  onMoveToRoot: (ids: string[]) => void;
  onUploadToRoot: (files: File[]) => void;
}) {
  const crumbs = breadcrumbs(currentPath);

  return (
    <nav
      aria-label="Folder path"
      className="flex flex-wrap items-center gap-1 text-body text-text-mute"
    >
      {crumbs.map((crumb, index) => {
        const isLast = index === crumbs.length - 1;
        const isRoot = index === 0;
        const label = isLast ? (
          <span className="px-1.5 py-0.5 text-text-hi">{crumb.label}</span>
        ) : (
          <button
            type="button"
            className="rounded-sm px-1.5 py-0.5 hover:text-text-hi hover:underline focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none"
            onClick={() => onNavigate(crumb.path)}
          >
            {crumb.label}
          </button>
        );

        return (
          <span key={crumb.path} className="flex items-center gap-1">
            {index > 0 ? (
              <ChevronRight className="size-3.5 text-text-faint" />
            ) : null}
            {isRoot ? (
              <DropZone
                aria-label="Build root (drop to move here)"
                getDropOperation={(types) =>
                  types.has(DRAG_TYPE) || types.has("file") ? "move" : "cancel"
                }
                onDrop={async (e) => {
                  const fileItems = e.items.filter(isFileDropItem);
                  if (fileItems.length > 0) {
                    const files = await Promise.all(
                      fileItems.map((item) => item.getFile()),
                    );
                    onUploadToRoot(files);
                    return;
                  }
                  const textItem = e.items.find(
                    (item) => isTextDropItem(item) && item.types.has(DRAG_TYPE),
                  );
                  if (!textItem || !isTextDropItem(textItem)) {
                    return;
                  }
                  const payload = JSON.parse(
                    await textItem.getText(DRAG_TYPE),
                  ) as DragPayload;
                  if (payload.ids.length > 0) {
                    onMoveToRoot(payload.ids);
                  }
                }}
                className={cn(
                  "rounded-sm outline-none transition-colors",
                  "data-[drop-target]:ring-2 data-[drop-target]:ring-ring data-[drop-target]:ring-inset",
                )}
              >
                {label}
              </DropZone>
            ) : (
              label
            )}
          </span>
        );
      })}
    </nav>
  );
}
