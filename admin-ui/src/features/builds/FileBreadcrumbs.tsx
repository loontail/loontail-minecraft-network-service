import { ChevronRight } from "lucide-react";
import { DropZone, isFileDropItem } from "react-aria-components";

import { breadcrumbs } from "@/features/builds/fileTree";
import { cn } from "@/shared/lib/cn";

// The Root crumb doubles as an upload drop target: OS files dropped on it upload
// to the build root.
export function FileBreadcrumbs({
  currentPath,
  onNavigate,
  onUploadToRoot,
}: {
  currentPath: string;
  onNavigate: (path: string) => void;
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
                aria-label="Build root (drop files to upload here)"
                getDropOperation={(types) =>
                  types.has("file") ? "move" : "cancel"
                }
                onDrop={async (e) => {
                  // react-aria's DropZone doesn't catch async onDrop errors, so a
                  // getFile() rejection would surface unhandled.
                  try {
                    const fileItems = e.items.filter(isFileDropItem);
                    if (fileItems.length === 0) {
                      return;
                    }
                    const files = await Promise.all(
                      fileItems.map((item) => item.getFile()),
                    );
                    onUploadToRoot(files);
                  } catch {}
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
