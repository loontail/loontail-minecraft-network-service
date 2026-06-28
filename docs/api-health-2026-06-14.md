# Loontail Launcher API — Health Report (2026-06-14)

> Live functional health check of `loontail-launcher-api` running on `:80`
> (origin behind the `cms.loontail.dev` Cloudflare tunnel), DB `loontail_app`
> on docker PG `:5433`. Method: full route inventory (114 routes) + launcher /
> admin contract cross-check + live probing of 9 endpoint groups.

## 1. Overall verdict

The API is **healthy and live**: infra, auth, Yggdrasil (account + Mojang
authserver/sessionserver), catalog, and bundle-registry all resolve at the paths
the launcher actually calls and reject unauthenticated/invalid requests cleanly
(401/403/422, no 5xx). The **only functional break is the texture (skin/cape)
surface**: the launcher + `@loontail/yggdrasil-client` target
`/api/yggdrasil/textures/*`, but the API serves textures at top-level
`/textures/*` — every skin/cape lookup, upload, and clear request 404s.
Separately, the new admin panel has **no skin/texture registry** (a
high-severity regression vs the old Strapi Yggdrasil plugin).

## 2. Status table

| Endpoint group | Path(s) probed | Live status | Key evidence |
|---|---|---|---|
| Infra | `GET /health`, `GET /metrics` | **OK** | 200 `{"database":"up","status":"ok"}`; Prometheus exposition with loontail_* counters |
| Auth — login | `POST /api/auth/login` | **OK** | empty→422 (missing username); bad creds→403 `forbidden`; exact launcher path |
| Auth — register | `POST /api/auth/register` | **OK** | empty→422 (missing username); not exercised with valid creds (would persist) |
| Auth — refresh | `POST /api/auth/refresh` | **AUTH_OK** | no/bogus bearer→401 `unauthorized`; session-rotation path |
| Auth — logout | `POST /api/auth/logout` | **OK** | no/bogus token→200 `{"ok":true}`, idempotent no-op |
| Auth — me | `GET /api/auth/me` | **AUTH_OK** | unauth→401; `POST`→405 (GET-only, registered). Served but launcher doesn't call it |
| Ygg authserver | `POST /api/yggdrasil/authserver/{authenticate,refresh,validate,invalidate}` | **AUTH_OK / OK** | bad creds→403 ForbiddenOperationException; invalidate→204; bogus subpath→404 (proves routes real); credential paths rate-limited (429) as designed |
| Ygg session/meta | `GET /api/yggdrasil` (+ `/`), `…/sessionserver/session/minecraft/hasJoined`, `…/profile/{uuid}`, `POST …/join`, `POST /api/yggdrasil/api/profiles/minecraft` | **OK / AUTH_OK** | meta doc 200; hasJoined no-query→400, with query→204; profile→204; join bad token→403; bulk-profiles (doubled-`/api`, the path the launcher builds)→200 `[]` |
| Catalog | `GET /api/clients` (+ full populate query, `/{id}`), `/api/keywords`(`/{id}`), `/api/servers`(`/{id}`) | **AUTH_OK** | all 401 `unauthorized` (player_session gated); nonexistent subpath→404 proves 401 is a real gate; legacy `/clients` (no `/api`)→404, launcher never calls it |
| Bundle registry — manifest | `GET /api/bundle-registry/builds/{slug}/manifest` | **AUTH_OK** | 401 for real + bogus slug (auth gate before lookup); root `/bundle-registry/…/manifest`→404 (manifest is `/api`-only) |
| Bundle registry — files | `GET /bundle-registry/builds/{slug}/files/{path}` | **AUTH_OK** | 401 unauth at top-level static origin (where manifest `url` fields point) |
| **Textures — launcher path** | `GET/PUT/DELETE /api/yggdrasil/textures/{uuid\|skin\|cape}` | **MISMATCH** | 404 with **empty body** (bare Axum fallback = route not registered) on every verb |
| **Textures — actual API path** | `GET /textures/{uuid}`, `PUT/DELETE /textures/{skin\|cape}`, `GET /textures/{uuid}/{skin\|cape}` | **OK / AUTH_OK** | lookup 200 `{}`; PUT/DELETE→401 JSON envelope (exists, needs Bearer); raw-PNG→404 JSON `not_found` (handler "no texture", route healthy) |
| Bulk-profiles single-`/api` | `POST /api/profiles/minecraft` | **NOT_IMPLEMENTED** | 404 — not served and **not called** by launcher (launcher uses doubled-`/api` path); not a defect |

## 3. Confirmed bugs / contract mismatches

### BUG-1 (P0) — Skin upload 404: `/api/yggdrasil/textures/skin` vs `/textures/skin`
- **Launcher calls:** `PUT /api/yggdrasil/textures/skin`, `multipart/form-data` with field `file` = `image/png` blob (`asset.png`) + field `variant` = `CLASSIC|SLIM`, `Authorization: Bearer <ygg accessToken>`. Origin: `@loontail/yggdrasil-client` `client.ts uploadSkin()` (`putMultipart` in `http.ts`) → launcher `src/main/services/skin/skin.ts uploadSkinYggdrasil`, with `apiRoot = API_URL + '/api/yggdrasil'` + endpoint `texturesSkin = '/textures/skin'`.
- **API serves:** `PUT /textures/skin` (top-level), `crates/textures/src/lib.rs` `handlers::upload` on `/{segment}`. Probe `/textures/skin` → **401** (exists, needs auth); the **body/field contract matches the handler exactly** — only the path prefix is wrong.
- **Observed:** launcher path → **404 empty body** (bare fallback); correct path → 401.
- **Root cause:** `apiRoot` defaults to `API_URL + '/api/yggdrasil'` (launcher `.env` doesn't set `YGGDRASIL_API_ROOT`); textures are mounted at top-level `/textures` (`crates/server/src/main.rs` L122), not under the Yggdrasil prefix.
- **Recommended fix (client/launcher, not API):** point the textures base at top-level `/textures` — decouple textures from the Yggdrasil-protocol `apiRoot` in `@loontail/yggdrasil-client`. Method (PUT) and multipart body are already correct — **do not** change them. (Alternative, if the API is the canonical contract owner / for deployed-launcher compat: also mount the textures router under `/api/yggdrasil/textures` as an alias.)

### BUG-2 (P0) — Texture lookup 404: `/api/yggdrasil/textures/{uuid}` vs `/textures/{uuid}`
- **Launcher calls:** `GET /api/yggdrasil/textures/{undashedUuid}`, expects `{skin?:{url,variant}, cape?:{url}}` (`TexturesLookupResponse`). Origin: client `getTextures()` → launcher `services/auth/yggdrasilClient.ts fetchTextures`; invoked by `verify.ts enrichYggdrasilAccount` and `skin/skin.ts` (pre/post-upload URL lookup). `apiRoot = API_URL + '/api/yggdrasil'` (config.ts:18) + endpoint `/textures` (endpoints.ts:11).
- **API serves:** `GET /textures/{uuid}` (`textures/src/lib.rs:64-66` `handlers::lookup`). Probe → **200 `{}`** unauthenticated (public lookup).
- **Observed:** launcher path → **404 empty body**; correct path → 200.
- **Root cause / fix:** same prefix drift as BUG-1; one base-path correction resolves lookup, upload (skin/cape), and delete (skin/cape) together.

### BUG-3 (P0, same root cause) — Cape upload + skin/cape clear all 404
- **Launcher calls:** `PUT /api/yggdrasil/textures/cape` (multipart, field `file` only, no variant), `DELETE /api/yggdrasil/textures/{skin|cape}`. Origin: client `uploadCape()/deleteSkin()/deleteCape()` → launcher `skin/skin.ts`.
- **API serves:** `PUT /textures/cape`, `DELETE /textures/{skin|cape}` — all probed at the top-level path → **401** (exist, need auth).
- **Observed:** every `/api/yggdrasil/textures/*` verb → **404 empty body**.
- **Root cause / fix:** identical to BUG-1/BUG-2 — fixing the textures base prefix repairs all five verbs (GET lookup, PUT skin, PUT cape, DELETE skin, DELETE cape) at once. Methods and bodies already match the server handlers.

> Net effect of BUG-1/2/3: **the entire skin/cape feature in the launcher is non-functional today** (every request 404s), despite the server-side textures crate being healthy and correctly auth-gated. This is a single-prefix client-config defect, not multiple unrelated breaks.

## 4. Admin gaps vs Strapi

| Gap | Severity | Detail | Recommendation |
|---|---|---|---|
| **Skin/Cape (texture) registry admin** | **HIGH** | Old Strapi Yggdrasil plugin had a full Textures admin (`TexturesPage` + `textures-admin.routes.ts`): list skins, list capes (paginated + search), upload skin (CLASSIC/SLIM)/cape, delete by id, 3D SkinViewer/2D preview, detail modal. New SPA nav (`admin-ui/src/App.tsx`) is Dashboard/Users/Catalog/Bundles/Logs only; `crates/admin/src/lib.rs` registers **no** textures routes; `crates/textures` exposes only the public per-UUID endpoints; `store.rs` has **no list/find_many**. Live: `GET /admin/textures/skins` → SPA HTML shell (route absent), vs real admin routes → JSON 401. | Add a Textures admin page (SPA) + `/admin/textures/*` routes in `crates/admin`, backed by new `store.rs` list/find_many over skins/capes. API + admin-UI work. |
| Texture orphan validate + purge-missing | MEDIUM | Strapi had `POST /yggdrasil/textures/validate` (rows whose file is missing) and `/purge-missing` (bulk cleanup). No equivalent → DB/disk drift undetectable/unrepairable from the panel. | Add admin maintenance endpoints + UI action under the new Textures page. |
| Admin upload skin/cape on behalf of a user | MEDIUM | Strapi let an admin set a user's skin/cape (base64 + variant, keyed by userId). New per-UUID `PUT /textures/{skin\|cape}` is the self-service/game path, not moderation; Users page has no skin/cape controls. | Add admin-initiated texture assignment/moderation (Users row + Textures page). |
| Yggdrasil sessions / issued-token list | LOW | Strapi's Sessions sub-page was an empty placeholder, so functional loss is minimal. New admin has no global sessions/tokens view; per-user `/admin/users/{id}/revoke-tokens` partially covers invalidation. | Optional global token/session list if operationally needed. |
| Strapi Users & Permissions / Content Manager / Media Library / version pickers | LOW | Generic Strapi surfaces (roles/permissions, media library, minecraft-version autocomplete) not reproduced; new Catalog/Users pages cover equivalent domain CRUD. | Accept as intentional consolidation unless a specific Strapi-only workflow is still required. |

## 5. Broken (5xx) / Not-implemented needing attention
- **No 5xx observed** on any well-formed request across infra, auth, Yggdrasil, catalog, or bundle-registry. The server is structurally sound.
- **NOT_IMPLEMENTED (benign):** `POST /api/profiles/minecraft` (single-`/api`) → 404. **Not a defect** — the launcher constructs the doubled-`/api` path `/api/yggdrasil/api/profiles/minecraft`, which is served and returns 200.
- The "needs attention" items are all 404 contract mismatches (BUG-1/2/3, texture surface), not server errors.

## 6. Prioritized fix list

**P0 — breaks a live launcher feature**
1. Fix the textures base-path so skin/cape calls hit top-level `/textures/*` instead of `/api/yggdrasil/textures/*` (resolves BUG-1/2/3 in one change). Either client-side (`@loontail/yggdrasil-client` decouple textures base + launcher rebuild) or an API-side `/api/yggdrasil/textures` alias mount (no launcher rebuild; compat for already-deployed launchers). Methods/bodies are already correct. Add a regression test asserting the resolved textures URL.

**P1 — operational regression vs Strapi**
2. Restore the **skin/cape registry admin** (HIGH gap): SPA Textures page + `/admin/textures/*` routes + `store.rs` list/find_many. Bundle in admin upload-on-behalf-of-user and validate/purge-missing (the two MEDIUM gaps) — same page/data layer.

**P2 — nice-to-have / accept-as-is**
3. Optional global Yggdrasil session/token list in admin (LOW; old page was a stub).

**Relevant paths:** API textures crate `crates/textures/src/lib.rs` (mount at `crates/server/src/main.rs` L122); admin routes `crates/admin/src/lib.rs`; admin SPA nav `admin-ui/src/App.tsx`; launcher skin flow `loontail-launcher/src/main/services/skin/skin.ts` and `…/services/auth/yggdrasilClient.ts`; client lib `loontail-yggdrasil` (`client.ts`, `http.ts`, `endpoints.ts`, `config.ts:18`).
