# Merge Catalog "Builds" with the Bundle Registry — Implementation Plan

Source of truth for the multi-wave effort that fuses the catalog "client/build"
domain with the bundle registry in the loontail-launcher-api admin panel.

## Goal (user)
1. Fix new-build creation: admins must be able to add background/title/poster
   images and publish — matching and exceeding the old legacy-backend media UX. **(Wave 0 — DONE)**
2. Embed the bundle INTO the build: the build page has a file-system widget to
   upload files; the build then IS a manifest with a working launcher link.
3. Net: one "Build" concept end-to-end (builds + bundle registry merged).

## Confirmed decisions
- **D1 = 1:1 owned bundle.** Each catalog client owns exactly one bundle (auto-created
  with the build). Reuse all existing bundle code (manifest/upload/storage) verbatim;
  add a real FK `catalog_clients.bundle_id -> bundles(id)`. "Bundle" disappears as a
  separate noun in the UI.
- **D6 = one Builds page.** Replace the Catalog→Clients tab + the Bundles page with a
  single "Builds" page (details + media + file widget + manifest link). Keywords/Servers
  stay accessible. Keep `crates/bundles` endpoints under the hood.
- **Sequencing:** hotfix first (Wave 0, done), then the merge.

## Frozen launcher contracts (DO NOT BREAK — `loontail-launcher` is a shipped consumer)
- `GET /api/clients` → `{clients:[...]}`; every client MUST keep a non-empty `slug`
  (slug-less clients are silently dropped). `bundleSlug` MUST remain present.
- Manifest: `GET /api/bundle-registry/builds/{slug}/manifest`, byte-exact
  (the launcher hashes the raw JSON for drift detection — do not change key order,
  whitespace, or conditional field omission in `crates/bundles/src/manifest.rs`).
- File bytes: `/bundle-registry/builds/{slug}/files/{*path}`, same-origin HTTPS.
- Auth everywhere is the session Bearer (`AuthUser`); admin writes are `AdminUser`.
- Additive-only changes to `ClientResponse` (launcher zod ignores unknown fields).
- `/api` is ONE grouped Router subtree in `crates/server/src/main.rs` — new public
  routes JOIN it, never add a second `.nest("/api", …)` (axum panics on overlap).

## Environment / verify loop
- Backend runs on **:80** (`.env`), DB `loontail_app` on dockerized PG `llapi-testdb` (:5433).
- Tests need PG: `DATABASE_URL=postgres://loontail:loontail@localhost:5433/loontail_test`.
- Backend gate: `cargo test --workspace` + `cargo build`. NOTE: active toolchain is
  rustc/clippy **1.95.0**, which fails `clippy -D warnings` + `fmt --check` on PRE-EXISTING
  repo code (new `doc_lazy_continuation` lint, fmt drift in textures tests). New code must
  be clean under `clippy -D warnings -A clippy::doc_lazy_continuation`. (Toolchain decision
  pending; do not let pre-existing drift block new-code verification.)
- admin-ui gate: `npm run build` (tsc + vite) + `npm test` from `admin-ui/`.
- Live: rebuild SPA (`npm run build`) → restart backend (`taskkill //IM loontail-launcher-api.exe //F`
  then run the binary) → Playwright at `http://localhost/admin` (admin / loontail123).
  The Vite dev proxy targets :8080, NOT our :80 — use the rebuilt-SPA path for live checks.

## Wave 0 — Hotfix (DONE, verified)
- Backend: `crates/catalog` — `repo::list_clients_admin` (no draft filter + `published`
  flag), `admin::list_clients`, `GET /admin/catalog/clients` route, `ClientAdminDto`/
  `ClientAdminList`. Tests: `crates/catalog/tests/admin_clients.rs` (2).
- Frontend: `useAdminClients`; `CatalogPage.tsx` reads the admin list, real Draft/Published
  badge + Publish/Unpublish, dialog stays open in edit mode after create (media uploads
  immediately), Publish action. Removed dead `useClients`.

## Wave 1 — Data-model link (backend + migration)
- New migration `00NN_catalog_bundle_link.sql`:
  - `ALTER TABLE catalog_clients ADD COLUMN bundle_id UUID REFERENCES bundles(id) ON DELETE SET NULL;`
  - Backfill `bundle_id` from `bundle_slug` where a matching `bundles.slug` exists (tolerate dangling).
  - Add `UNIQUE(bundle_id, relative_path)` to `bundle_artifacts` — PRE-CHECK for dup rows first
    (closes the racy SELECT-then-INSERT upsert gap). Skip/guard if dups exist.
  - NEVER edit existing migration files (sqlx checksums applied migrations).
- `crates/catalog`: thin read of the linked bundle; `build_client_dto` inlines an additive
  `bundle: { slug, version, status, filesCount, manifestUrl } | null` (looked up via `bundle_id`).
  Keep `bundleSlug` (derive from the linked bundle on read; keep collapse-empty→null).
  Cross-crate: add a minimal `catalog -> bundles` read dependency or a shared read fn (avoid a cycle).
- Tests: extend `crates/catalog/tests/contract.rs` — assert nested `bundle` present/null,
  `bundleSlug` still present, draft still hidden, AuthUser still required.

## Wave 2 — Build create auto-provisions a bundle
- `create_client` (and the merged create path) creates the owned `bundles` row + sets
  `bundle_id` atomically (bundle slug defaults to the client slug; collision-safe). Accept a
  `publish` flag. Return `{ id, bundleSlug }`.
- Tests: create → owned bundle exists → manifest 404-until-ready; publish reconciliation
  (decide: warn-but-allow publishing a build whose bundle isn't `ready`).

## Wave 3 — Merged Build page: Details + Media (frontend)
- New `admin-ui/src/features/builds/api.ts` (compose catalog + bundles hooks). New `/builds`
  list + `/builds/{slug}` full page (PageHeader + SectionTabs). Register route in `App.tsx`,
  nav in `AppShell.tsx` `NAV`.
- Details tab: auto-slug from title, version fields, available, Publish/Unpublish with real
  state, copyable Manifest URL panel.
- Media tab: poster/background/titleImage + screenshots, rendered unconditionally (the row
  exists before this page is reached). Beat the legacy backend: drag-drop onto slots, in-place replace,
  required poster+background, screenshot reorder via `sortOrder`.

## Wave 4 — File-system widget (frontend; wires existing backend)
- Files tab: nested folder tree from `relativePath.split('/')` (client-side; backend
  unchanged), breadcrumb, drag-drop single-file upload with `targetPath`, create folder,
  rename/move, rehash, per-file SHA-256 copy chip, download-once switch, multi-select bulk
  delete, search, "remove missing" via `validate`, ZIP upload retained, Regenerate/Validate,
  status + `lastGeneratedAt`. Wire the already-present unwired hooks in
  `admin-ui/src/features/bundles/api.ts` (useUploadFile/useCreateFolder/useRenameFile/
  useRehashFile/useBulkDelete/useToggleDownloadOnce/useUpdateBuild).

## Wave 5 — Decommission separate pages
- Remove the `Bundles` nav entry + page; fold Catalog→Clients into `Builds`. Keep `crates/bundles`
  endpoints (merged UI uses them). Verify `loontail-launcher` still parses `/api/clients` and
  syncs a manifest unchanged (load-bearing compat check).

## Per-wave gate
Backend: `cargo build` + `cargo test --workspace` on PG:5433; new code clippy-clean
(`-A clippy::doc_lazy_continuation`). Frontend: `npm run build` + `npm test`. Live: Playwright
against the rebuilt SPA on :80.
