// Drag payload carrying moved artifact ids + source paths (for the descendant
// guard). Only the FileBreadcrumbs Root DropZone still uses react-aria DnD.
export const DRAG_TYPE = "application/x-loontail-build-entry";

export interface MoveRequest {
  ids: string[];
  /// Destination folder relativePath ("" = build root).
  targetDir: string;
}
