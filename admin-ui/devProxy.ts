// Dev-server proxy table for `vite dev`, kept out of vite.config.ts so it is
// testable without loading the vite/esbuild plugin chain.
import type { IncomingMessage } from "node:http";

const BACKEND = "http://localhost:8080";

// Backend prefixes that live under the SPA's own base. Mirrors the mounts in
// crates/server/src/main.rs. Listed individually because vite's base is /admin/,
// so a bare /admin proxy would shadow the dev server's own asset serving.
const ADMIN_API_PREFIXES = [
  "/admin/auth",
  "/admin/users",
  "/admin/catalog",
  "/admin/bundles",
  "/admin/textures",
  "/admin/analytics",
  "/admin/logs",
];

// Backend prefixes outside the SPA base — no client route can collide with these.
const ROOT_PREFIXES = ["/api", "/textures", "/catalog-media", "/bundle-registry"];

// why: some of the prefixes above are also App.tsx client routes (/admin/users,
// /admin/textures, /admin/logs). Proxying a browser navigation there would answer
// from the backend's compile-time-embedded index.html, whose hashed asset URLs do
// not exist on the dev server — a blank page on deep link or hard reload. GET
// navigations therefore stay on vite (its SPA fallback serves the dev shell);
// everything else (fetch/XHR, images, non-GET) reaches the backend.
export function bypassHtmlNavigation(req: IncomingMessage): string | undefined {
  if (req.method !== undefined && req.method !== "GET") {
    return undefined;
  }
  return req.headers.accept?.includes("text/html") ? req.url : undefined;
}

export const devProxy: Record<
  string,
  string | { target: string; bypass: typeof bypassHtmlNavigation }
> = {
  ...Object.fromEntries(
    ADMIN_API_PREFIXES.map((prefix) => [
      prefix,
      { target: BACKEND, bypass: bypassHtmlNavigation },
    ]),
  ),
  ...Object.fromEntries(ROOT_PREFIXES.map((prefix) => [prefix, BACKEND])),
};
