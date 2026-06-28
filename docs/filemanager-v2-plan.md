Confirmed: SPA is embedded via rust-embed from `admin-ui/dist`; `build.rs` has `cargo:rerun-if-changed=../../admin-ui/dist`. After `npm run build`, rebuilding the server re-embeds; touching `crates/admin/src/spa.rs` forces a recompile if cargo doesn't pick up the change. All facts are verified. I have everything needed to produce the final plan.

---

# FINAL IMPLEMENTATION PLAN — Build File Manager redo + MC→Forge/Fabric/Java recommended cascade

Both features live in `e:/workspace/elixir/loontail-minecraft-network-service/admin-ui/`. Verified against the actual code; every load-bearing claim below was confirmed (Checkbox signature, Forge maven ordering, the two existing test files, the v1 seed, kit APIs, rust-embed path).

## Critic improvements applied (with provenance)

| # | Critic point | Resolution in this plan |
|---|---|---|
| C1 | Shift-range dead zone / Checkbox can't carry `shiftKey` (verified: `onCheckedChange:(v:boolean)=>void`, no event) | Both table and grid use a clickable **wrapper** that owns `onClick` (carries `e.shiftKey`) with a **decorative** (non-interactive) `<Checkbox>` inside. No shiftKey is ever routed through the interactive Checkbox. §FM-3/4. |
| C1b | Table double-fire (td onClick + inner interactive Checkbox both toggle) | Eliminated: the checkbox cell renders the **decorative** Checkbox; only the wrapper `<button>`/cell owns the toggle. One handler per click. §FM-3. |
| C2 | Lying counter ("5 selected → delete 3") | Selection is **restricted to artifact-backed entries only** (folders are not selectable via checkbox). `selectedCount` then always equals `selectedArtifactIds.length`. Header select-all selects only artifact children. §FM-2/6. |
| C3 | Touch / no-hover: selection + kebab invisible | Checkbox column and kebab are `opacity-0` only behind `@media (hover:hover)`; on coarse pointers they are always visible. Long-press on row/card opens the context menu. §FM-3/4/8. |
| C4 | File-name single-click silently downloads (footgun) | File **name selects** the row (cheap, reversible). Download moves to the kebab + SelectionToolbar (already present). Only **folder** names navigate. Distinct affordance: folder name is a link (underline+pointer), file name is plain text. §FM-1. |
| C5 | Grid vs list selection location differs | Grid corner checkbox is **persistent** (same visibility rule as the table column), identically styled, so toggling views never relocates selection. §FM-4. |
| C6 | No in-flight feedback on async row/bulk actions | Download fires a toast; bulk delete/move/rehash already disable via `pending` props on dialogs/toolbar. Toast added on single-file download path. §FM-6. |
| C7 | Existing `BuildFilesTab.test.tsx` (13 tests, `role="grid"`+`dblClick`+row-click) breaks | **Rewrite in scope** as an explicit work item (Wave 1). §FM-9, Wave 1. |
| C8 | Cascade is "tunable magic" / Fabric special-cased | **One committed rule**, prose matches sketch: on MC change, Forge keep-if-valid-else-recommended/latest; Fabric & Java keep-if-set-else-seed-recommended. Stated as final, not tunable. §VC-4D. |
| C9 | Recommended Badge inside `SelectItem` pollutes trigger text / breaks typeahead | `SelectItem` gets explicit `textValue` (bare value) + a trailing `Star` icon with `aria-label`; Badge rendered in a flex row, not in value text. Test asserts trigger shows bare value. §VC-4B/C. |
| C10 | Java select loses free-text escape hatch for unknown majors | A **"Custom…"** item swaps the select for an inline `<Input>` so an admin can target a Java major the generator hasn't seen. `withCurrent` still preserves stored legacy values. §VC-4B. |
| C11 | Forge recommended fallback inverted (verified: maven order is oldest-first; `firstSeen`=oldest) | Mirror `pickForge`: `recommended[mc].forge = isRecommended ?? isLatest ?? builds[last]` (newest). Forge arrays sorted newest-first before emit. §VC-2. |
| C12 | `MINIMAL_FALLBACK` + committed seed lack `java`/`recommended`/`version` | Both updated to v2 shape with non-empty `java:[25,21,17,16,8]`. §VC-2/6. |
| C13 | v1→v2 normalization spread order wrong (`...data` last) | Fixed: `{ ...data, java: data.java ?? [], recommended: data.recommended ?? {}, version: data.version ?? 1 }`. §VC-3. |
| C14 | Existing `BuildDetailPage.test.tsx` `SAMPLE_VERSIONS` cast `as VersionsCatalog` will fail when fields become required | Fixture updated with `java`/`recommended`/`version`; existing 1.7.10 legacy cascade test reconciled to the new rule. §VC-7, Wave 4. |
| C15 | a11y after dropping RAC (roving-tabindex/type-ahead lost) | Table gets `aria-label`, each name button an accessible name, checkboxes `aria-label`, focus order checkbox→name→kebab; one keyboard test added. §FM-1/10. |
| C16 | `staleTime: Infinity` means long sessions never see enriched catalog | Documented as a conscious decision (build-time asset); no change. §VC-3 note. |
| C17 | Dual root-drop (new wrapper + FileBreadcrumbs DropZone) may double-fire | Upload drop-wrapper handles **only the table/grid body**; `FileBreadcrumbs` Root DropZone keeps root drops. They cover disjoint regions. `dndHooks.ts` trimmed but `DRAG_TYPE` kept (still used by FileBreadcrumbs). §FM-8/9. |
| C18 | Playwright synthetic dblclick does NOT trigger RAC `onAction` | The whole redesign is single-click; verification uses single-click only. Wave 5. |

---

## FEATURE 1 — BUILD FILE MANAGER

**Decision (unchanged, critic-approved): replace RAC `GridList` with native `<button>`/`<table>` + checkbox.** Verified root cause in `FileList.tsx`/`FileGrid.tsx`: `selectionBehavior="replace"` + `onAction`-bound open + a `slot="drag"` `AriaButton` on every artifact row — exactly the trio that makes a single click do nothing and double-click-to-open unreliable.

### FM-1. Interaction contract (the rule — every click does one obvious thing)

| Target | Single click |
|---|---|
| Row/card checkbox **wrapper** | Toggle that entry in `selectedKeys`; `shiftKey` extends a range. Never navigates/opens. Only artifact-backed entries have a checkbox. |
| **Folder** name (real `<button>`, link affordance: `hover:underline cursor-pointer`) | Navigate into folder (`onAction(path)` → `openEntry` → `navigate`). |
| **File** name (plain `<button>`, NOT link-styled) | **Select** the row (toggle membership), same as its checkbox. Download is NOT on the name. |
| Header checkbox | Select-all / clear-all of **artifact-backed children** of the current folder. |
| Kebab (⋮) | Open `FileContextMenu` at the button rect (existing `openMenu`). Holds Open / Download / Rename / Move to… / download-once / Rehash / Delete. |
| Right-click / long-press on row/card | Open `FileContextMenu` at cursor. |
| Breadcrumb crumb | Navigate to ancestor (unchanged `FileBreadcrumbs`). |

There is no double-click anywhere. Folders open on a single name click; files select on a single name click; download is always via the kebab or the SelectionToolbar "Download". Keyboard: native — Tab order is checkbox → name → kebab; Enter/Space on name opens-or-selects; Enter/Space on the checkbox toggles.

### FM-2. `BuildFilesTab.tsx` adapter (edit, ~40 lines net)

- **Default view:** line 165 `useState<ViewMode>("grid")` → `"list"`.
- **Selection is artifact-only.** Replace `onSelectionChange(keys: Selection)` (lines 260-266) with:

```ts
const lastClickedRef = useRef<string | null>(null);
const selectableChildren = useMemo(
  () => children.filter((c) => c.artifact),         // folders carry no id → not selectable
  [children],
);

function toggleOne(relativePath: string, shiftKey: boolean) {
  setSelectedKeys((prev) => {
    const next = new Set(prev);
    if (shiftKey && lastClickedRef.current) {
      const order = selectableChildren.map((c) => c.relativePath);
      const a = order.indexOf(lastClickedRef.current);
      const b = order.indexOf(relativePath);
      if (a !== -1 && b !== -1) {
        const [lo, hi] = a < b ? [a, b] : [b, a];
        for (let i = lo; i <= hi; i++) next.add(order[i]);
        lastClickedRef.current = relativePath;
        return next;
      }
    }
    next.has(relativePath) ? next.delete(relativePath) : next.add(relativePath);
    lastClickedRef.current = relativePath;
    return next;
  });
}
function toggleAll(checked: boolean) {
  setSelectedKeys(checked ? new Set(selectableChildren.map((c) => c.relativePath)) : new Set());
}
const allSelected = selectableChildren.length > 0 &&
  selectableChildren.every((c) => selectedKeys.has(c.relativePath));
const someSelected = !allSelected && selectableChildren.some((c) => selectedKeys.has(c.relativePath));
```

- Because only artifact-backed entries enter `selectedKeys`, `selectedCount` (line 327) now equals `selectedArtifactIds.length` (lines 324-326) and the bulk-delete dialog count (line 567) — the lying counter is gone. Keep `navigate()` clearing selection (lines 235-238).
- Replace `useBuildFilesDnd`/`dragAndDropHooks`/`isDropTarget` wiring (lines 253-258, 461-477) with the new prop set (FM-3) and an upload drop-wrapper (FM-8).

### FM-3. `FileList.tsx` (rewrite) — shadcn `Table`

Use `@/components/ui/table` and the **decorative** `Checkbox` (no `onCheckedChange`). Props:

```ts
interface FileViewProps {
  entries: TreeEntry[];
  selectedKeys: Set<string>;
  onToggle: (relativePath: string, shiftKey: boolean) => void;
  onToggleAll: (checked: boolean) => void;
  allSelected: boolean;
  someSelected: boolean;
  onAction: (relativePath: string) => void;          // folder→navigate, file→select (FM-1)
  onOpenMenu: (entry: TreeEntry, position: { x: number; y: number }) => void;
}
```

Five columns; `<Table aria-label="Build files">`:

| # | Header | Width | Cell |
|---|---|---|---|
| 1 | header checkbox wrapper `<button aria-label="Select all" onClick={()=>onToggleAll(!allSelected)}>` with decorative `<Checkbox checked={allSelected} className={someSelected?"opacity-60":undefined}/>` | `w-10` | for artifact entries: `<button type="button" aria-label={`Select ${name}`} onClick={(e)=>{e.stopPropagation(); onToggle(path, e.shiftKey)}}>` wrapping decorative `<Checkbox checked={selectedKeys.has(path)}/>`. Empty cell for folders. Visibility: `opacity-0 group-hover:opacity-100` only under `@media(hover:hover)`; always visible on coarse pointers and when selected. |
| 2 | "Name" | `1fr` | `<button type="button" onClick={()=>onAction(path)}>` + icon + truncated name. Folder: `hover:underline text-text-hi cursor-pointer` (link affordance) + `Folder` icon `text-text-mute`. File: `text-text-hi` (no underline) + `File` icon `text-text-faint`. |
| 3 | "Size" (right) | `w-28` | `formatBytes(artifact.size)` or `""` for folders. `tabular-nums text-text-mute`. |
| 4 | "Once" (right) | `w-20` | `downloadOnce ? "Yes" : ""`. |
| 5 | (empty) | `w-12` | Kebab `<Button variant="ghost" size="icon" aria-label={`Actions for ${name}`}>` (MoreVertical). Same hover rule as col 1. `onClick` → `openMenu(entry, rect)`. |

Row: `<TableRow className="group" data-state={selected?'selected':undefined} onContextMenu={cursor→openMenu}>`; long-press handler (pointerdown timer) → `openMenu`. shadcn `data-[state=selected]:bg-muted` gives a clearly visible selected background; add `data-[state=selected]:shadow-[inset_2px_0_0] data-[state=selected]:shadow-primary` for a left accent. The table renders only when `children.length>0` (EmptyState stays in `BuildFilesTab`).

### FM-4. `FileGrid.tsx` (rewrite) — card grid

`grid grid-cols-[repeat(auto-fill,minmax(11rem,1fr))] gap-3`. Each card `<div className="group relative">` with two click zones:

- **Card body** `<button type="button" onClick={()=>onAction(path)}>`: large icon (size-7), name (truncate), subtitle (`"Folder"` / `formatBytes`). Folder body uses link affordance, file body plain.
- **Corner checkbox wrapper** (`absolute left-2 top-2`, artifact entries only): `<button aria-label={`Select ${name}`} onClick={(e)=>{e.stopPropagation(); onToggle(path, e.shiftKey)}}>` wrapping decorative `<Checkbox checked={selectedKeys.has(path)}/>`. **Persistent** (same `@media(hover:hover)` rule + always visible when selected) so selection lives in the same place across views.
- **Corner kebab** (`absolute right-1 top-1`): same `aria-label`/handler as table.
- Selected style: `data-[selected] → border-primary bg-accent-soft ring-1 ring-primary`. `onContextMenu` + long-press → `openMenu`.

No `slot="drag"`, no RAC, no internal move-drag in the grid (Move stays reachable via kebab + SelectionToolbar).

### FM-5. KEEP verbatim (10)
`fileTree.ts`, `FileContextMenu.tsx`, `FileBreadcrumbs.tsx` (its per-crumb `<button>` and Root `DropZone` are RAC-independent and keep working), `FileManagerToolbar.tsx` (New menu, Grid/List toggle wired to `viewMode`, Regenerate/Validate), `SelectionToolbar.tsx`, `MoveDialog.tsx`, `NewFolderDialog.tsx`, `RenameDialog.tsx`, `download.ts`, all 13 mutations in `bundles/api.ts`. `components/ui/table.tsx` + `components/ui/checkbox.tsx` reused as-is (decorative variant already supported).

### FM-6. In-flight feedback
Single-file download (kebab/SelectionToolbar) fires `toast.message(`Downloading ${name}…`)`. Bulk delete/move/rehash already gate on `pending` (ConfirmDialog/MoveDialog/SelectionToolbar). No click appears to do nothing.

### FM-7. Move + upload
- **Move:** always via kebab "Move to…" and SelectionToolbar "Move" → existing `MoveDialog`. No DnD required.
- **Upload drop-wrapper:** a thin `<div onDragOver onDrop>` around the table/grid **body** in `BuildFilesTab`: on `drop` with `dataTransfer.files` → existing `uploadFiles(files, currentPath)`; shows `ring-2 ring-ring` while dragover. `FileBreadcrumbs` Root DropZone keeps move-to-root/upload-to-root. Disjoint regions → no double-fire.
- **Upload button path** (FileManagerToolbar hidden inputs) fully preserved.

### FM-8. `dndHooks.ts` (trim)
Delete `useBuildFilesDnd` (no longer consumed). **Keep** `DRAG_TYPE` (still imported by `FileBreadcrumbs.tsx`) and `MoveRequest` (used by `BuildFilesTab.onMove`). Remove `DragPayload`/`isSelfOrDescendant`/RAC imports if unused after the trim.

### FM-9. File ledger
- **Rewrite (2):** `src/features/builds/FileList.tsx`, `src/features/builds/FileGrid.tsx`.
- **Edit (2):** `src/features/builds/BuildFilesTab.tsx` (FM-2 + drop-wrapper), `src/features/builds/dndHooks.ts` (trim).
- **Rewrite test (1):** `src/features/builds/BuildFilesTab.test.tsx` — see Wave 1.
- **Keep verbatim (10):** FM-5 list.
- **Add (2 tests):** `FileList.test.tsx`, `FileGrid.test.tsx`.

### FM-10. Tests
- `BuildFilesTab.test.tsx` rewrite: `getByRole("grid")` → `getByRole("table", {name:/build files/i})`; folder-open `user.dblClick(row)` → `user.click(name button)`; selection `user.click(row)` → `user.click(checkbox by aria-label "Select mods")`; keep the upload-input selector `input[type=file]:not([accept])` and kebab `Actions for <name>` accessible names (FileManagerToolbar + new components preserve them); update the grid/list toggle test to assert the **table** appears in list mode and cards in grid mode. Expected final count ≈ 13 (1:1 migration).
- `FileList.test.tsx`: folder name → `onAction`; checkbox → `onToggle(path,false)` and NOT `onAction`; **shift-click checkbox** extends a range; header checkbox → `onToggleAll(true)`; kebab opens menu; one keyboard test (Tab to name, Enter opens; Space on checkbox toggles).
- `FileGrid.test.tsx`: card body single click opens; corner checkbox toggles without opening; shift-range on corner checkbox.

---

## FEATURE 2 — VERSION CASCADE

### VC-1. `versions.json` v2 shape
Backwards-compatible superset of v1; `version: 2`.
```jsonc
{
  "version": 2,
  "minecraft": [{ "id": "1.21.4", "type": "release" }, ...],   // newest-first (unchanged)
  "fabric": ["0.16.10", ...],                                  // newest-first (unchanged)
  "forge": { "1.21.4": ["54.1.6", "54.1.5", ...], ... },       // NOW newest-first (VC-2)
  "java": [25, 21, 17, 16, 8],                                 // NEW: distinct majors, desc
  "recommended": {                                             // NEW: per-MC picks
    "1.21.4": { "java": 21, "forge": "54.1.6", "fabric": "0.16.10" }, ...
  },
  "generatedAt": "..."
}
```
- `recommended[mc].java` = `manifest.javaVersion.majorVersion` (fallback 8). Only for resolved MCs; unresolved MCs fall back to `java[0]` at the UI.
- `recommended[mc].forge` = newest-meaningful Forge (VC-2); `null` if no Forge.
- `recommended[mc].fabric` = `fabric[0]`.

### VC-2. `generate-versions.mjs` (edit)
- **Forge ordering + recommended (verified fix).** Maven order is oldest-first and `pickForge` precedence is `recommended → isLatest → builds[last]` (newest). So:
  ```js
  const forge = {};
  const forgeRecommended = {};
  const byMc = new Map();
  for (const b of forgeBuilds) {
    if (!byMc.has(b.minecraftVersion)) byMc.set(b.minecraftVersion, []);
    byMc.get(b.minecraftVersion).push(b);
  }
  for (const [mc, builds] of byMc) {
    const newestFirst = [...builds].reverse();              // maven oldest-first → newest-first
    forge[mc] = newestFirst.map((b) => b.forgeVersion);
    const rec = builds.find((b) => b.isRecommended)
            ?? builds.find((b) => b.isLatest)
            ?? builds[builds.length - 1];                    // newest, mirrors pickForge
    forgeRecommended[mc] = rec.forgeVersion;
  }
  ```
- **Bounded Java resolve.** `const RESOLVE_LIMIT = Number(process.env.VERSIONS_JAVA_LIMIT ?? 30);` — resolve only the newest `RESOLVE_LIMIT` releases, concurrency pool of 6, per-MC `try/catch` (a 404 ⇒ no java for that MC, never a build failure). The kit caches manifests (`minecraft-manifest:<id>:<sha1>`).
- **Prior-seed merge (incremental).** Read existing `public/versions.json`; reuse `prior.recommended[mc].java` so prior majors survive a smaller run.
- **Java option list:** `java = [...new Set([...resolvedMajors, 25,21,17,16,8])].sort((a,b)=>b-a)`.
- **Fabric recommended:** `recommended[mc].fabric = fabric[0]` for every MC key.
- **Seed-fallback preserved exactly:** `keepExistingOrSeed` + `process.exit(0)` unchanged.
- **`MINIMAL_FALLBACK` → v2:** add `version:2`, `java:[25,21,17,16,8]`, `recommended:{}` so even a fresh-checkout total-failure write is v2-shaped.
- Uses `kit.versions.minecraft.resolve({ version: asMinecraftVersionId(m.id) })` and `r.manifest.javaVersion?.majorVersion ?? 8` (both verified).

### VC-3. `useVersions.ts` (edit)
```ts
export interface RecommendedVersions { java?: number; forge?: string | null; fabric?: string | null; }
export interface VersionsCatalog {
  version?: number;
  minecraft: MinecraftVersionEntry[];
  fabric: string[];
  forge: Record<string, string[]>;
  java: number[];
  recommended: Record<string, RecommendedVersions>;
  generatedAt: string;
}
const EMPTY_CATALOG: VersionsCatalog = {
  minecraft: [], fabric: [], forge: {}, java: [], recommended: {},
  generatedAt: new Date(0).toISOString(),
};
```
queryFn normalization (spread order fixed — overrides AFTER spread):
```ts
const data = (await res.json()) as Partial<VersionsCatalog>;
return { ...data, java: data.java ?? [], recommended: data.recommended ?? {}, version: data.version ?? 1 } as VersionsCatalog;
```
`staleTime: Infinity` retained — conscious decision (build-time asset; a long admin session sees enrichment only after reload).

### VC-4. `BuildDetailPage.tsx` (edit) — four dropdowns + Java select + cascade

**A. Recommended lookup (in `BuildDetailsTab`):**
```ts
const rec = catalog?.recommended?.[form.minecraftVersion] ?? {};
const recForge = rec.forge ?? undefined;
const recFabric = rec.fabric ?? undefined;
const recJava = rec.java;  // number | undefined
```

**B. Java select replaces the free-text `<Input>` (lines 310-318), with escape hatch:**
```ts
const javaList = (catalog?.java ?? []).map(String);
const javaOptions = withCurrent(javaList, form.runtimeVersion);  // legacy "11","1.8" survive
const CUSTOM = "__custom__";
const [javaCustom, setJavaCustom] = useState(false);
```
Render `<Select aria-label="Java version">` (always enabled — Java is meaningful without MC) with the `NONE` sentinel, each option `value={v} textValue={v}` and a trailing `Star` (aria-label `recommended`) when `String(recJava)===v`, plus a `<SelectItem value={CUSTOM}>Custom…</SelectItem>`. Picking `CUSTOM` sets `javaCustom=true` and swaps in an `<Input>` so an admin can type a major the generator hasn't seen yet.

**C. Recommended marker in Forge / Fabric / Java (NOT MC):** explicitly meeting "recommended in each of the three **dependent** dropdowns" (MC is the user's primary choice; left unbadged by design). Each `SelectItem` keeps a bare `textValue` and renders `{value}` + (when it equals the recommended) a `Star` icon in a flex row — never injected into value text, so the trigger shows a clean value and typeahead still works. When Forge has no `-recommended`/`-latest` promo, the newest build (`forge[mc][0]`, now newest-first) is what `recForge` points to, so the dropdown still surfaces a sensible marked default.

**D. Cascade on MC change (`setMinecraft`, lines 199-212) — ONE final rule (prose matches code):**
```ts
function setMinecraft(next: string) {
  setForm((prev) => {
    if (next === "") return { ...prev, minecraftVersion: "", forgeVersion: "", fabricVersion: "" };
    const r = catalog?.recommended?.[next] ?? {};
    const forgeList = catalog?.forge?.[next] ?? [];
    return {
      ...prev,
      minecraftVersion: next,
      // Forge: keep prior only if still valid for the new MC, else recommended/newest.
      forgeVersion: forgeList.includes(prev.forgeVersion) ? prev.forgeVersion : (r.forge ?? ""),
      // Fabric (MC-independent): keep an explicit pick; seed recommended only when empty.
      fabricVersion: prev.fabricVersion || (r.fabric ?? ""),
      // Java: keep an explicit/legacy pick; seed recommended only when empty.
      runtimeVersion: prev.runtimeVersion || (r.java !== undefined ? String(r.java) : ""),
    };
  });
}
```
Rule, stated for the PR: **Forge** keep-if-valid-else-recommended; **Fabric & Java** keep-if-set-else-seed-recommended. Never silently overwrite a still-valid Forge pick or a user-typed Java.

**E. Disabled-until-MC:** Forge/Fabric keep their `disabled={!mcChosen}` (unchanged); Java stays **always enabled** (placeholder `—`).

**F. Submit:** unchanged — `runtimeVersion: nullable(form.runtimeVersion)` sends the same `"21"` string shape; backend contract untouched.

### VC-5. Aesthetic
All four reuse the imported `Select`/`SelectTrigger`/`SelectContent`/`SelectItem`. Recommended marker = lucide `Star` (`size-3 text-primary`, `aria-label="recommended"`) already in the icon-import pattern, or `Badge variant="outline"`. No new tokens; the 2-col grid is unchanged (Java cell swaps `<Input>`→`<Select>`/custom-`<Input>`).

### VC-6. File ledger
- **Edit (3):** `scripts/generate-versions.mjs` (VC-2), `src/features/builds/useVersions.ts` (VC-3), `src/pages/BuildDetailPage.tsx` (VC-4).
- **Edit seed (1):** `public/versions.json` — hand-seed v2 (`version:2`, non-empty `java:[25,21,17,16,8]`, `recommended` for the top entries already present in the file), so CI/Docker (kit absent) ships a usable Java dropdown + badges. Regeneration enriches it later.
- **Keep:** `withCurrent`, `NONE`, the Radix pointer/scroll polyfills.

### VC-7. Tests (`BuildDetailPage.test.tsx`, edit)
Extend `SAMPLE_VERSIONS` (currently lines 29-40) with `version:2, java:[21,17], recommended:{ "1.21.4":{java:21,forge:"54.1.6",fabric:"0.16.10"}, "1.20.1":{java:17,forge:"47.4.0",fabric:"0.16.10"} }`. New/updated cases:
- Java select is a combobox `name:"Java version"` showing stored `"21"`; lists catalog majors; legacy `runtimeVersion:"11"` preserved via `withCurrent`; "Custom…" swaps in an input.
- Recommended marker present for `54.1.6` (Forge), `0.16.10` (Fabric), `21` (Java) inside the open listbox **and** the trigger shows the bare value after selecting it (typeahead-safety assertion).
- Changing MC 1.21.4→1.20.1 sets Forge `47.4.0` (prior `null`→recommended) and seeds Java `17` when empty; assert trigger text after `selectOption`.
- Regression: an MC with no `recommended.java` falls back to `java[0]` as the default.
- Reconcile the existing legacy-MC test (1.7.10, lines 266-283) to the new `setMinecraft` rule (legacy MC has no recommendation ⇒ dependents unchanged; value still injected via `withCurrent`).

---

## WAVE PLAN (with verification)

Working dir for npm: `e:/workspace/elixir/loontail-minecraft-network-service/admin-ui/`.

**Wave 0 — Branch + baseline.** Initialize/confirm git (repo currently not a git repo per env); branch `feature/filemanager-and-version-cascade`. Run baseline `npm install` and `npm run test` to record the green starting count.

**Wave 1 — File manager render + adapter (TDD).**
1. Rewrite `BuildFilesTab.test.tsx` to the new contract (FM-10) — it now fails (red).
2. Rewrite `FileList.tsx` (FM-3) and `FileGrid.tsx` (FM-4); add `FileList.test.tsx`/`FileGrid.test.tsx` (FM-10).
3. Edit `BuildFilesTab.tsx` (FM-2: default list, artifact-only selection, `toggleOne`/`toggleAll`/`allSelected`/`someSelected`, drop-wrapper) and trim `dndHooks.ts` (FM-8).
- **Verify:** `npm run test` green (file-manager suites incl. shift-range, lying-counter-fixed, single-click open, keyboard, touch-visible); `npm run build` typechecks.

**Wave 2 — versions.json v2 generator + seed.**
1. Edit `generate-versions.mjs` (VC-2). 2. Hand-seed `public/versions.json` to v2 (VC-6).
- **Verify (offline):** `VERSIONS_JAVA_LIMIT=0 npm run versions` keeps the seed and exits 0 (no kit/network). **Verify (with kit):** `npm run versions` writes `version:2` + non-empty `java` + `recommended` with **newest-first** forge arrays and recommended = newest-meaningful; spot-check one MC with no `-recommended` promo points to its newest build, not oldest.

**Wave 3 — useVersions typing.** Edit `useVersions.ts` (VC-3).
- **Verify:** `tsc -b` clean; a unit assertion that a v1 payload (no `java`) normalizes to `java:[]`.

**Wave 4 — BuildDetailPage dropdowns + cascade (TDD).**
1. Update `BuildDetailPage.test.tsx` `SAMPLE_VERSIONS` + add cases (VC-7) — red. 2. Edit `BuildDetailPage.tsx` (VC-4).
- **Verify:** `npm run test` green (version-cascade suite incl. trigger-clean-value + custom-Java + MC-change defaults + java[0] fallback + legacy 1.7.10).

**Wave 5 — Full build, re-embed, live Playwright.**
1. `npm run build` (runs `prebuild` generator within the bounded budget). 2. Rebuild server so rust-embed picks up `admin-ui/dist`: `cargo build -p loontail-admin` (or workspace build); if cargo doesn't detect the asset change, `touch crates/admin/src/spa.rs` to force recompile (verified embed at `crates/admin/src/spa.rs` `#[folder=".../admin-ui/dist"]`, with `cargo:rerun-if-changed=../../admin-ui/dist` in `build.rs`). 3. Run the server, drive admin on `:80` with Playwright.
- **Playwright checks (single-click only — synthetic dblclick does NOT trigger RAC `onAction`, and the redesign no longer uses it):** single-click folder **name** navigates; single-click file **name** selects (visible highlight); checkbox selects with a visible row background; shift-click extends a range and the SelectionToolbar count matches what Delete removes; kebab + right-click menu open; bulk Move/Delete/Download; upload via button **and** drag-drop; grid/list toggle keeps selection in place. On Builds → a build → Details: MC/Forge/Fabric/Java dropdowns render, Forge/Fabric/Java show a Recommended marker, the Java select shows a clean trigger value, changing MC auto-fills recommended dependents.

### Cross-cutting gates
- `npm run test` green (all suites, incl. rewritten `BuildFilesTab.test.tsx` and extended `BuildDetailPage.test.tsx`).
- `npm run versions` offline keeps seed + exit 0; with kit writes v2 (`java`+`recommended`, newest-first forge).
- `npm run build` completes within the bounded generator budget; server rebuild re-embeds the new SPA.
- Live Playwright on `:80` confirms every click does something on a plain single click.

### Concrete absolute paths
File manager: `e:/workspace/elixir/loontail-minecraft-network-service/admin-ui/src/features/builds/{FileList.tsx,FileGrid.tsx,BuildFilesTab.tsx,dndHooks.ts,BuildFilesTab.test.tsx,FileList.test.tsx,FileGrid.test.tsx,FileContextMenu.tsx,FileBreadcrumbs.tsx,FileManagerToolbar.tsx,SelectionToolbar.tsx,MoveDialog.tsx,NewFolderDialog.tsx,RenameDialog.tsx,fileTree.ts,download.ts}`, `.../src/components/ui/{table.tsx,checkbox.tsx}`, `.../src/features/bundles/api.ts`.
Version cascade: `e:/workspace/elixir/loontail-minecraft-network-service/admin-ui/scripts/generate-versions.mjs`, `.../src/features/builds/useVersions.ts`, `.../src/pages/BuildDetailPage.tsx`, `.../src/pages/BuildDetailPage.test.tsx`, `.../public/versions.json`.
Kit APIs (verified): `e:/workspace/elixir/minecraft-kit/src/versions/minecraft.ts` (`resolve().manifest.javaVersion?.majorVersion`), `.../src/versions/forge.ts` (`list()[].isRecommended`/`isLatest`; maven order oldest-first via `.../src/core/xml.ts`; `pickForge` precedence recommended→latest→last), `.../src/index.ts` (`asMinecraftVersionId`).
Re-embed: `e:/workspace/elixir/loontail-minecraft-network-service/crates/admin/src/spa.rs`, `.../crates/admin/build.rs` (embeds `admin-ui/dist`).