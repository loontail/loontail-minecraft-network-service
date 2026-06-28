# Builds follow-up — Round 2 plan

Follow-up to `merge-builds-bundles-plan.md`. Driven by an investigation workflow + user decisions.

## Locked decisions
- **Keywords + Servers move INTO Builds, per-build.** No global Catalog/Settings page — DELETE `CatalogPage` + its nav/route. Manage a build's keywords + servers from the Build detail page (attach existing via combobox + create-new inline + detach). Keep the global `catalog_keywords`/`catalog_servers` tables + the `catalog_client_keywords`/`catalog_client_servers` join tables (the launcher `Client` DTO inlines `keywords[]`/`servers[]` — frozen contract). "Not global" = no global management page; the entities stay shared in the DB but are reached through a build.
- **Versions** → build-time `admin-ui/scripts/generate-versions.mjs` (imports `@loontail/minecraft-kit` as a **devDependency**, calls the 4 resolvers the old Strapi controller used, writes `public/versions.json`); cascading shadcn `Select` pickers (Forge/Fabric gated on chosen MC). `runtimeVersion` stays free-text (kit's runtime API is OS/arch-coupled via `node:os`). `@loontail/minecraft-kit` must NOT enter the browser bundle (Node-only: tsup platform:node, re-exports node:child_process/fs/os).
- **File manager** → `react-aria-components` `Tree` + `useDragAndDrop` (Apache-2.0, React 19 peer, headless → fits shadcn, native drop-onto-folder). Move == rename to a new path prefix (server already supports recursive rename of files AND folders).
- Do all 6 waves now, verify each.

## Verify loop / gotchas (apply every wave)
- Backend tests: `YGGDRASIL_PUBLIC_URL=/api/yggdrasil DATABASE_URL=postgres://loontail:loontail@localhost:5433/loontail_test cargo test ...` (the local `.env` absolute YGGDRASIL_PUBLIC_URL breaks 2 textures tests otherwise). Gate new Rust with `clippy -D warnings -A clippy::doc_lazy_continuation` (rustc 1.95 flags pre-existing doc lints).
- **Added/edited a migration** → `touch crates/core/src/db.rs` (sqlx::migrate! embeds at compile time) then rebuild, else it silently doesn't apply.
- **Edited admin-ui** → `npm run build` in admin-ui, then `taskkill //IM loontail-launcher-api.exe //F`, `touch crates/admin/src/spa.rs` (rust-embed compile-time embed), `cargo build -p loontail-server`, restart. Confirm served `index-<hash>.js` changed. Vite dev proxy targets :8080 not :80 — use the rebuilt-SPA path for live checks.
- admin-ui gate: `npm run build` + `npm test`. Live: Playwright at http://localhost/admin (admin / loontail123).
- Adding an npm dep: also `npm install --package-lock-only` + commit the lockfile (CI uses `npm ci`).

## Waves

### Wave A — P0 data-safety (backend)
- **P0-a delete leaks bundle+files**: `catalog::admin::delete_client` does a bare `DELETE`; FK `bundle_id ON DELETE SET NULL` orphans the owned `bundles` row + `bundle_artifacts` + on-disk `{BUNDLES_STORAGE_ROOT}/builds/{slug}/`. Fix: read `bundle_id`/`bundle_slug`, in ONE sqlx tx delete the client AND the owned bundle (rows) + delete files on disk; add reusable `loontail_bundles::delete_owned_bundle(pool, storage_root, bundle_id)` (reuse existing delete-build-files + delete-bundle). Guard NULL bundle_id (legacy). Fix `BuildsPage.tsx` delete ConfirmDialog copy.
- **P0-b create not transactional**: `create_client` commits before provisioning the bundle. Fix: provision + link `bundle_id`/`bundle_slug` INSIDE the existing tx (pass `&mut *tx`); create the on-disk dir after commit (idempotent) so rollback leaves no stray dir.
- P2: correct the `bundle_summary` doc comment; make `repo::upsert_artifact` a real `INSERT ... ON CONFLICT (bundle_id, relative_path) DO UPDATE` preserving `download_once`.
- Tests: delete removes bundle+artifacts+dir; failed provision rolls back the client.

### Wave B — Catalog → Builds (keywords/servers per-build) + P1 fixes
- Backend (`crates/catalog`): add DETACH endpoints `DELETE /admin/catalog/clients/{client_id}/keywords/{keyword_id}` and `.../servers/{server_id}` (delete the join row; 204). Keep attach/create/list. P1: auto-provision a bundle in `update_client` when `bundle_id IS NULL` (mirror create's find-or-create) so legacy/null-bundle builds get one; if `bundleSlug` changed, re-resolve `bundle_id` via `find_bundle_id_by_slug`.
- Frontend: DELETE `pages/CatalogPage.tsx` + its test + the `/catalog` route (App.tsx) + the Catalog nav entry (AppShell.tsx). In `BuildDetailPage.tsx` add per-build **Keywords** + **Servers** management (a new tab e.g. "Servers & Tags", or sections): list `client.keywords`/`client.servers`, attach existing (combobox from `useKeywords`/`useServers` minus attached) + create-new inline + detach. Add `useDetachKeyword`/`useDetachServer` to `features/catalog/api.ts`; reuse `useAttachKeyword`/`useAttachServer`/`useCreateKeyword`/`useCreateServer`. After attach/detach/create, invalidate the admin clients query so `client.keywords/servers` refresh.
- P1 stale filesCount: bundle file mutations must also invalidate `catalogKeys.clients()` so the Builds list + manifest panel refresh — do it in the Builds-feature consumer layer (don't import catalogKeys into features/bundles to avoid a cyclic dep; e.g. wrap in BuildFilesTab or add an invalidator).
- P2: remove dead bundles hooks (`useBuilds`/`useCreateBuild`/`useUpdateBuild`/`useDeleteBuild`/`useDiskSpace`) after confirming no caller (keep `useBuild` + the file-op hooks).

### Wave C — Version dropdowns (build-time versions.json)
- `admin-ui/scripts/generate-versions.mjs` imports `@loontail/minecraft-kit`, emits `public/versions.json` ({ minecraft:[], fabricFor(mc), forgeFor(mc) } — mirror the 4 Strapi resolvers + cascade). Add `@loontail/minecraft-kit` as devDependency; wire `prebuild`/`build` in package.json; refresh lockfile.
- `BuildDetailPage.tsx`: replace the 4 free-text version Inputs with cascading shadcn `Select`s (MC list; Forge/Fabric disabled until MC chosen, filtered by MC). Keep `runtimeVersion` free-text. No schema change (fields already `string|null`). Allow a custom/empty value so legacy versions not in the list still display.
- Verify the bundle has no `node:` leak (build succeeds).

### Wave D — File-manager backend hardening + move endpoints (crates/bundles)
- Reusable `move_subtree(tx, bundle_id, old_rel, new_rel)` (disk rename + child-row prefix rewrite + category re-derive) wrapped in ONE sqlx tx (close the non-atomic gap). DB-aware conflict check (artifact_exists_at + any_artifact_with_prefix) → clean **409** instead of unique-index 500. Guard folder-into-own-subtree (`new.starts_with(old + "/")`).
- New endpoints: `POST /builds/{slug}/files/{entryId}/move {targetDir}` and `POST /builds/{slug}/files/move {ids, targetDir}` (multi-move, one manifest regen at end). DTOs camelCase. Every op ends via `repo::regenerate_manifest` (never hand-edit artifacts.json).
- Tests: move across top-level dir updates `category`; multi-move; self-into-descendant → 4xx; collision → 409.

### Wave E — File-manager UI (react-aria-components Tree)
- Add `react-aria-components` dep (+ lockfile). Extend `fileTree.ts` with `buildTree(artifacts)` (nested; node id = relativePath; folders accept drops). New tree component in `features/builds/`. Two-pane Card: react-aria `Tree` sidebar (recursive, expandable, multi-select) + the existing table as the current-folder pane. `useDragAndDrop`: getItems → dragged relativePath(s); onItemDrop(folder, items) → call the new move/move-bulk endpoint (single atomic call); shouldAcceptItemDrop → target is a folder & not a descendant; onRootDrop → move to ""; keep external-file drop → upload. Reuse all existing mutations from DnD handlers.
- Verify with Playwright `browser_drag`/`browser_drop`.

### Wave F — Final audit sweep + verification
- Full `cargo test` (PG:5433, YGGDRASIL_PUBLIC_URL=/api/yggdrasil) + admin-ui build+test + a full Playwright pass on :80: build delete removes files; create rolls back cleanly; no Catalog page; per-build keywords/servers; version dropdowns cascade; file-manager DnD move; manifest bytes stable for unaffected builds.
