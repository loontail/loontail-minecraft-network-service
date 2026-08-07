// The byte stream lives at the root file route (NOT the `/api/` prefix), guarded
// by the AuthUser session cookie, so a plain same-origin anchor click downloads it.

function basename(relativePath: string): string {
  const parts = relativePath.split("/");
  return parts[parts.length - 1] ?? relativePath;
}

// Per-segment encoding so spaces / unicode names escape without escaping the slashes.
function encodeRelativePath(relativePath: string): string {
  return relativePath.split("/").map(encodeURIComponent).join("/");
}

export function downloadFile(slug: string, relativePath: string): void {
  const href = `/bundle-registry/builds/${encodeURIComponent(
    slug,
  )}/files/${encodeRelativePath(relativePath)}`;
  const anchor = document.createElement("a");
  anchor.href = href;
  anchor.download = basename(relativePath);
  anchor.rel = "noopener";
  document.body.appendChild(anchor);
  anchor.click();
  anchor.remove();
}
