/// The custom drag payload type carrying the moved artifact ids (and the source
/// relativePaths, for the descendant guard). Still used by the FileBreadcrumbs Root
/// DropZone, the only remaining react-aria DnD surface — the table/grid views moved
/// to native click selection + the Move dialog, so the grid/list no longer drag.
export const DRAG_TYPE = "application/x-loontail-build-entry";

export interface MoveRequest {
  ids: string[];
  /// Destination FOLDER relativePath ("" = build root).
  targetDir: string;
}
