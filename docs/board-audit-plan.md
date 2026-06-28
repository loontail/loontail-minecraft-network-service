# Admin board audit — fixes + design refinement (round 3)

From a 4-agent audit. User asks: fix photo upload, style the dropdowns, deep bug audit + fixes, significantly improve design. Keep the dark shadcn system; kill raw native controls.

## Verify loop (every wave)
- Rust: `YGGDRASIL_PUBLIC_URL=/api/yggdrasil DATABASE_URL=postgres://loontail:loontail@localhost:5433/loontail_test cargo test ...`; new code `clippy -D warnings -A clippy::doc_lazy_continuation`.
- admin-ui: `npm run build` + `npm test`. Adding an npm dep → `npm install --package-lock-only` + commit lockfile.
- Live: rebuild server + `touch crates/admin/src/spa.rs` (re-embed SPA) + restart; Playwright at localhost/admin (admin/loontail123).

## P0 (bugs)
- **P0-1** photo upload: `ClientMediaSection.tsx` file `accept="image/png,image/jpeg,image/webp"` (both inputs) hides the user's photo in the OS picker → change to `accept="image/*"`; backend stays validator.
- **P0-2** size cap + invisible rejection: `crates/catalog/src/lib.rs:31` `MAX_MEDIA_UPLOAD_BYTES = 8 MiB` → **32 MiB**; the route `DefaultBodyLimit` (lib.rs:67) must be **strictly > handler cap** (cap + 1 MiB headroom) so the in-handler JSON 400 ("image is too large (max 32 MB)") wins over axum's opaque 413; reword the message (admin.rs).
- **P0-3** orphan media: `crates/catalog/src/admin.rs::delete_client` removes DB rows + bundle dir but NOT `{catalog.storage_root}/{client_hex}/` → after `tx.commit()` best-effort `remove_dir_all` of the client media dir (post-commit, mirror the bundle fix).

## P1 (native controls + bugs)
- **P1-1** Select primitive: `npm i @radix-ui/react-select`; new `components/ui/select.tsx` (mirror dropdown-menu.tsx styling, ChevronDown, Check, z above Dialog z-50); migrate the 5 native `<select>` in BuildDetailPage (MC/Fabric/Forge + add-keyword + add-server); delete `SELECT_CLASS`. Preserve placeholder, disabled-until-MC, `withCurrent()` legacy values.
- **P1-2** FileUpload: new `components/shared/FileUpload.tsx` (hidden input + outline Button w/ Upload icon + filename + optional dashed dropzone w/ drag-over highlight; routes drag+pick through one mutate path; resets input.value on settle). Migrate the media slots.
- **P1-3** Textarea primitive: `components/ui/textarea.tsx` (mirror input.tsx); replace the inline `<textarea>` in BuildDetailPage.
- **P1-4** data-loss: BuildDetailPage tabs render via ternary that UNMOUNTS inactive tabs; `BuildDetailsTab` form is local state → edits lost on tab switch. Fix: lift Details form state to BuildDetailPage OR render all tabs always-mounted with `hidden`. (Same trap on inline keyword/server forms.)
- **P1-5** form never re-syncs: `useState(()=>buildToForm(build))` inits once → stale after save / when navigating build A→B (router reuses instance). Fix: `key={build.id}` on BuildDetailsTab (+ media/servers panels keyed by build.id) or useEffect re-sync.
- **P1-6** multi-screenshot upload drops all but first: ScreenshotsGallery input has no `multiple` and reads `files[0]` → add `multiple` + iterate.
- **P1-7** file-manager phantom folder: after delete/move of the current folder, `currentPath` points nowhere → validate against `data.artifacts` in onSuccess, fall back to nearest existing ancestor or "".
- **P1-8** PageHeader everywhere: Users/Textures/Dashboard hand-roll headers → extend `components/shared/PageHeader.tsx` (description + actions) and route all 5 pages through it.
- **P1-9** dedupe ConfirmDialog: merge `pages/users/ConfirmDialog.tsx` into `components/shared/ConfirmDialog.tsx` (description: ReactNode + width prop); repoint UserRowActions; delete the dup.

## P2 (polish/hardening)
- GIF support in `sniff_image` + `content_type_for` (magic `GIF8`); type-scale retune of primitives onto text-h2/body/caption; `SegmentedControl` shared component; dead-code sweep; build toasts wording; security: `serve_media` canonicalize+containment (keep 404), last-admin guard (refuse final delete/block/demote → 409), constant-time CSRF compare, logout tolerates stale 401.

## Waves (execution)
- **A (backend, parallel):** P0-2, P0-3, P2 GIF + security hardening (serve_media, last-admin, CSRF, logout). crates/{catalog,core,server,...}. Tests for: >8≤32MiB upload ok; delete removes media dir; GIF 201; oversized → JSON 400 not 413; last-admin guard 409; serve_media traversal still 404.
- **B1 (frontend core, parallel with A):** add `@radix-ui/react-select` + lockfile; new `select.tsx`, `textarea.tsx`, `FileUpload.tsx`; migrate ClientMediaSection (accept image/*, FileUpload, multiple screenshots, Replace/Remove polish); BuildFilesTab phantom-folder fix + tree dropzone. Does NOT touch BuildDetailPage.
- **B2 (frontend, after B1):** BuildDetailPage — migrate 5 selects → `<Select>`, textarea → `<Textarea>`, fix tab-data-loss (lift/always-mount) + form re-sync (`key={build.id}`). Update tests to Radix listbox semantics.
- **C (frontend cohesion, after B2):** PageHeader unification, ConfirmDialog dedupe, type-scale retune, SegmentedControl, dead-code.
- **Verify:** full cargo + npm gates + one live Playwright pass (upload large JPG/GIF, styled dropdowns above dialog, edit-tab-switch keeps edits, delete-build removes media dir).
