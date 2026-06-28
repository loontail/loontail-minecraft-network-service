# File manager → Google Drive + load-bug + site sweep (round 4)

From a 3-agent investigation. User: (1) file manager sometimes doesn't load, (2) make it look/work like Google Drive, (3) full-site bug sweep (e.g. builds-list extra container padding). Build CUSTOM on the already-installed `react-aria-components` (GridList + useDragAndDrop) + Radix `dropdown-menu`; no new deps if avoidable. Dark monochrome (GDrive IA, not its blue). Reuse ALL existing bundle mutations.

## Verify loop
- admin-ui: `npm run build` + `npm test`. New npm dep → `npm install --package-lock-only` + commit lockfile.
- backend: `YGGDRASIL_PUBLIC_URL=/api/yggdrasil DATABASE_URL=postgres://loontail:loontail@localhost:5433/loontail_test cargo test ...` + clippy `-A clippy::doc_lazy_continuation`.
- Live: rebuild server + `touch crates/admin/src/spa.rs` (re-embed) + restart; Playwright at localhost/admin (admin/loontail123). Backend running on :80, PG llapi-testdb :5433.

## LOAD BUG (P0) — infinite skeleton on dangling-bundle builds
Root cause: build with `bundle_id NULL` + leftover `bundle_slug="dsfsdfsd"` → DTO `bundle=null` but `bundleSlug="dsfsdfsd"`; `BuildDetailPage` passes the UNVERIFIED `build.bundleSlug` → `useBuild("dsfsdfsd")` → 404; `BuildFilesTab` guard `if (isLoading || !data)` has no error branch → skeleton forever.
- **Layer A (BuildDetailPage):** use `build.bundle?.slug ?? null` (drop the `?? build.bundleSlug`) in 3 places: BuildFilesTab prop (~:880), ManifestPanel (~:116), Details save bundleSlug (~:228). Dangling → null → no-bundle state; healthy → verified slug. Save with `bundleSlug:null` makes `link_owned_bundle` re-derive from build slug → heals.
- **Layer B (BuildFilesTab):** add an `isError` branch BEFORE the loading branch → retryable EmptyState ("This build points to a bundle that no longer exists" on 404).
- **Layer C (heal CTA):** BuildFilesTab prop becomes `build: ClientAdmin`; the no-bundle EmptyState gets a "Set up file storage" button → `useUpdateClient({ id, slug, ...current fields, bundleSlug:null })` → re-provisions + links (ensure_bundle_dir post-commit) → Files loads. Zero backend.
- **Layer D (backend repo.rs build_client_dto ~:318):** emit `bundle_slug: bundle.as_ref().map(|b| b.slug.clone())` (only when verified). Kills dangling slugs for ALL consumers incl. launcher. Verify the catalog contract test + launcher tolerate `bundleSlug:null` on published (already required for never-linked builds).
- Do NOT add a provision endpoint (update_client is one) or a SQL-only backfill (can't create the on-disk dir).

## GOOGLE-DRIVE FILE MANAGER (rewrite BuildFilesTab)
Download is the only new logic: file bytes at ROOT `/bundle-registry/builds/{slug}/files/{*path}` (AuthUser cookie; NOT the `/api/` prefix). No zip endpoint → multi-download loops single files, skips folders w/ toast.
- KEEP `fileTree.ts` (childrenOf/breadcrumbs/joinPath/parentPath/buildTree). DELETE `FileTreeView.tsx` after porting its DnD (DRAG_TYPE payload, shouldAcceptItemDrop, isSelfOrDescendant, onItemDrop upload/move, onRootDrop).
- REWRITE `BuildFilesTab.tsx` as a thin orchestrator: state `currentPath`, `selectedKeys:Set<Key>` (=relativePath), `viewMode:"grid"|"list"`, dialog flags; keeps useBuild + currentPath self-heal effect + childrenOf + nodesById/buildTree + selectedArtifactIds. Renders Toolbar → Breadcrumbs → SelectionToolbar(when sel>0) → FileGrid/FileList → processingError banner + FooterStatus. Prop = `build: ClientAdmin`. Adds isError + heal-CTA states.
- ADD: `FileManagerToolbar.tsx` (left: "New" Radix menu → New folder/Upload file/Upload ZIP w/ hidden inputs; right: grid/list toggle + "More" menu w/ Regenerate+Validate). `FileBreadcrumbs.tsx` (Root/mods/config, click=navigate, root drop target). `FileGrid.tsx` (default; RAC GridList, `grid-cols-[repeat(auto-fill,minmax(11rem,1fr))]` cards) + `FileList.tsx` (toggle; header + `grid-cols-[1fr_7rem_5rem_auto]` rows). `dndHooks.ts` (`useBuildFilesDnd` wrapping useDragAndDrop; only FOLDER cards are internal drop targets, targetDir=folder.relativePath; file branch→upload; text branch→move w/ self-descendant guard). `SelectionToolbar.tsx` ("{n} selected" + clear + Move + Download + Delete). `MoveDialog.tsx` (destination folder picker from buildTree, self/descendant disabled). `FileContextMenu.tsx` (one Radix dropdown-menu used by ⋮ AND onContextMenu at cursor; Open/Download/Rename/Move to…/Toggle download-once/Rehash/sep/Delete; implied folders (artifact===null) disable mutating items). `download.ts` (anchor w/ encodeURIComponent per segment).
- MOVE OUT RenameDialog + NewFolderDialog into own files. Reuse ConfirmDialog/EmptyState/FooterStatus/StatusBadge/ShaCell. All 13 mutations reused.
- Interactions: double-click/Enter opens folder(navigate) or downloads file; single-click selects (`selectionBehavior="toggle"`), Ctrl/Shift multi; drag card→folder = move; drag OS files = upload (full-area drop overlay via RAC isDropTarget); right-click + ⋮ = context menu; "New" menu. Tokens: bg-surface-1/2 cards, border-edge-md, data-[selected]:bg-accent-soft, data-[drop-target]:ring-ring.
- States: null-bundle→heal CTA; loading→grid skeletons; error→retryable; empty folder→FolderOpen EmptyState.
- Risk gates: RAC GridList single-click-select vs double-click-open must not conflict; cursor-positioned Radix menu (fallback: add `@radix-ui/react-context-menu` + lockfile); download must use root path + encodeURIComponent.
- Rewrite `BuildFilesTab.test.tsx` for `role="grid"` semantics + toggle + MoveDialog + context-menu download + OS-file drop.

## SITE BUGS
- **P1 #2 builds-list padding:** `BuildsPage.tsx` wraps the full-bleed Table in `<Card><CardContent>` (card py-6 + CardContent px-6 stacking on cell px-4 py-3). Replace with the canonical `<div className="rounded-lg border border-edge bg-card">` (as Users/Textures do); render empty/loading as a full-width colSpan row inside the table. Recheck BuildsPage.test.tsx.
- **P1 #3** same Table-in-padded-CardContent in `pages/logsTraffic/LiveLogsSection.tsx` + `TrafficSection.tsx` "Top paths" → standardize.
- **P1 #4** `ClientMediaSection.tsx` leading `<Separator/>` + redundant "Media" heading flush to card top → remove (tab already labeled, BuildDetailPage wraps it in a Card).
- **P2:** #5 standardize empty/loading/error on shared EmptyState+TableSkeleton across list pages; #6 dedup useDebounced/formatDate/shortUuid/SkeletonRows/StateRow (Users+Textures) → shared/lib + shared/lib/format.ts; #7 focus-visible ring on TrafficSection MetricSelector + SectionTabs; #8 SectionTabs aria-controls/role=tabpanel (+ aria-hidden inactive panels); #9 Draft/Hidden badges differentiate by icon/shape not hue; #10 BuildsPage CreateBuildDialog route all closes through handleOpenChange; #11 BuildDetailPage loading uses card-shaped skeletons not TableSkeleton; #12 trim `///`/decorative comments in FileUpload/ClientMediaSection; #13 TexturesPage merge the two setPage(1) effects.

## Waves (execution)
- **Backend (Layer D)** ∥ **GDrive rewrite (load-bug A/B/C + Wave 3)** — disjoint (cargo vs admin-ui files BuildFilesTab/BuildDetailPage/new).
- Then **site sweep (Wave 1 padding #2/#3/#4 + Wave 4 P2 #5-#13)** — admin-ui, sequential after the GDrive agent (shared npm build state; disjoint files: BuildsPage/logs/Users/Textures/ClientMediaSection/shared).
- Then rebuild + restart + live Playwright (load bug gone, GDrive UX, padding parity).
