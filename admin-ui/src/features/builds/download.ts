// Build a file-download anchor at the ROOT bundle-registry file route (NOT the
// `/api/` prefix): the byte stream lives at
// `/bundle-registry/builds/{slug}/files/{*path}` and is guarded by the AuthUser
// session cookie, so a plain same-origin anchor click downloads it. Each path
// segment is `encodeURIComponent`'d and rejoined so spaces / unicode names in the
// relative path are escaped without escaping the separating slashes.

/// The last path segment (display name) used as the suggested download filename.
function basename(relativePath: string): string {
  const parts = relativePath.split("/");
  return parts[parts.length - 1] ?? relativePath;
}

/// Encode each segment of a forward-slash relative path, preserving the slashes.
function encodeRelativePath(relativePath: string): string {
  return relativePath.split("/").map(encodeURIComponent).join("/");
}

/// Trigger a browser download of a single build file by `relativePath`.
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
