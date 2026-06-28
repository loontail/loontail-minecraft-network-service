Confirmed. `file_url` and `file_path` are stored per-row and keyed by `profile_uuid`. The verifier's note on BUG-2 is accurate: fix (b) alone leaves `file_url` embedding the old UUID, so the join-by-user-id approach (fix a) or also re-deriving `file_url` is the fully-correct path. I have everything needed.

---

# FINAL FIX PLAN — loontail-launcher-api (backend + admin SPA)

## SECURITY SUMMARY (do these first)

The single most important hardening is fixing the **rate limiter's proxy blindness (SEC-1)**: behind the documented compose/reverse-proxy deployment, `into_make_service_with_connect_info` always injects the proxy's address as the peer, so the per-IP login limiter collapses into one global bucket — any one client can exhaust the default 10/60s budget and lock out every user's login/register/yggdrasil-authenticate, and per-source brute-force isolation is gone. Fix it with an explicit, opt-in `TRUSTED_PROXY` (peer-as-today when unset; right-most trusted hop of XFF/X-Real-IP when set). This also closes **SEC-3** (never trust client XFF except behind a trusted proxy; fail-closed on credential paths). Pair it with: **SEC-2** gate `/metrics` behind `AdminUser`/metrics-token or an internal bind; **SEC-8** apply `nosniff` (+HSTS) globally and a fixed Content-Type on user-uploaded file routes so an uploaded `.html/.svg` cannot be sniffed/rendered in-origin; **SEC-10** strip `?token=` from tracing spans (WS session tokens leak into logs/proxy access logs). These are the high-leverage, low-blast-radius wins.

---

## CONFIRMED FIX LIST (de-duped, by severity)

Note on duplicate IDs in the source data: the backend and admin-ui audits each used their own `PERF-1`, `DUP-1..5`, `DEAD-1`. I disambiguate below with a crate/UI prefix.

### P0 — Security / correctness (ship first)

| id | title | files | concrete fix | why |
|---|---|---|---|---|
| **BUG-1** | Friend/presence queries decode nullable `minecraft_uuid` into non-`Option` `String` → 500 | `crates/network/src/friends.rs:53,59` (FriendRequestRow), `crates/network/src/presence.rs:218` (FriendPresenceRow) | Change `from_minecraft_uuid`/`to_minecraft_uuid` and `minecraft_uuid` to `Option<String>`; drop the `Some(...)` wrapper where mapped into `UserDto.minecraft_uuid`. Mirror `users.rs:147` which already uses `Option<String>`. | Any credential-only friend (registered, never bootstrapped) carries `NULL` (migration 0003) and currently 500s every friends/requests/presence endpoint; tests miss it because `seed_user` always bootstraps a UUID. |
| **SEC-1** | Per-IP auth limiter collapses to one bucket behind reverse proxy (login DoS + brute-force bypass) | `crates/server/src/ratelimit.rs:95-104,119-131`; `crates/server/src/main.rs:28-29,75-83`; new `crates/server/src/ip.rs` | Add opt-in `TRUSTED_PROXY` flag / trusted-CIDR config. When set, derive client IP from the right-most trusted hop of `X-Forwarded-For` (or `X-Real-IP`); when unset, use the transport peer as today. Never trust XFF unless trusted. | Behind the documented compose deployment the peer is always the proxy → whole internet shares one 10/60s bucket → trivial login lockout + no per-source throttling. |
| **SEC-3** | Limiter fails OPEN and falls back to client-controllable XFF | `crates/server/src/ratelimit.rs:99-104,129-131` | Fold into the SEC-1 fix: only derive IP from XFF behind a trusted proxy; on credential paths where IP truly can't be resolved, fail-closed (or apply a conservative global cap) instead of unthrottled passthrough. | Without this, a naive XFF "fix" lets an attacker rotate the header for a fresh bucket per request, bypassing the limiter entirely. |

### P1 — Security hardening / architecture / UX / a11y

| id | title | files | concrete fix | why |
|---|---|---|---|---|
| **SEC-2** | `/metrics` unauthenticated, leaks live operational telemetry | `crates/server/src/main.rs:130`; `crates/server/src/infra.rs:32-76` | Gate `/metrics` behind `AdminUser` or a dedicated metrics token, or bind to an internal listener / restrict by source network at the proxy. Optionally return only minimal status from `/health` to anonymous callers. | Anonymous callers get real-time network activity (heartbeats, relay bytes, active relays/users) — free recon/sizing oracle. (Telemetry, not creds — P1 defensible but real.) |
| **BUG-2** | Skin/cape rows keyed by `user_id` but read by `profile_uuid`; reconcile rewrites `profile_uuid` → texture silently orphaned | `crates/textures/src/handlers.rs:37-48,74`; `crates/core/src/identity.rs:308-322`; `crates/yggdrasil/src/profile.rs:55,64` | **Prefer fix (a):** make `users.profile_uuid` authoritative and have textures `lookup`/`read_png` join `skins/capes` via `users.id` (resolve `users WHERE profile_uuid=$1`, then by `user_id`). This also avoids the stale `file_url`/`file_path` embedding that fix (b) alone leaves wrong. If (a) is too invasive, do fix (b) **plus** re-derive `file_url` (`UPDATE skins/capes SET profile_uuid=$2, file_url=… WHERE user_id=$1` inside `assign_or_reconcile_profile_uuid`). | After register → launcher login → upload skin → in-game bootstrap → next login, reconcile rewrites `profile_uuid` and the texture row's stale UUID makes the skin appear deleted while the signed GameProfile still references it. Multi-step but real. |
| **BUG-3** | `PATCH /world-sessions/:id status='closed'` skips relay/presence/player-count cleanup that DELETE does | `crates/network/src/worlds.rs:86-172` (update) vs `:174-223` (close) | Factor `close()`'s cleanup (close `relay_sessions`, reset host presence to online + null `current_world_session_id`, zero `current_players`) into a shared helper; call it from `update()` when status transitions `open→closed`, inside a transaction. | A PATCH-close leaves `active` relay rows + stale host presence + inflated `current_players`; the FoF admission gate (`is_friend_of_active_member`, `join_requests.rs:68-97`) does **not** check `ws.status='open'`, so the policy gate can still pass for a closed world. (Note: the presence LATERAL does filter `ws.status='open'`, and `try_admit_player` requires open — so the real damage is the policy-gate leak + stale state, not a literal admission.) |
| **ARCH-1** | Analytics write seam (`record_event`) stranded in admin crate, dead in production | `crates/admin/src/analytics.rs:140-153` (write), `:97-135` (read) | **Recommendation (needs a write-vs-delete decision):** the dashboard bootstraps/new-users graphs render empty forever because nothing writes `user_events`. Pick one: **(write)** move `record_event` down into `core` (e.g. `core::analytics` or an `AppState` method) and have `POST /users/bootstrap` + register spawn it off the hot path, keeping the READ aggregation in admin; **(delete)** if the request_logs-based dashboard supersedes `user_events`, remove `record_event` + the read path + migration as dead code. Default suggestion: **write path** (small, restores a shipped feature). Medium risk (touches hot paths). | `admin` cannot be a dependency of domain crates, so the documented "domains emit events" can never happen; the module doc is provably false. |
| **AUTH-1** | No global 401 handler: expired session spams toasts, never redirects to login | `admin-ui/src/shared/api/queryClient.ts:5-19`; `admin-ui/src/shared/api/client.ts:99-109`; `admin-ui/src/features/auth/api.ts:12-31` | Add a global `QueryCache`/`MutationCache` `onError` (or a 401 hook inside `apiFetch`) that, on a 401 for any non-login call, runs `queryClient.setQueryData(authKeys.me, null)` + invalidates `authKeys.me` so `RequireAuth` redirects. Exempt the login call (LoginPage handles its own 401). | When the cookie expires mid-session, `me` stays stale 60s with `refetchOnWindowFocus:false`, so the app stays mounted and every mutation emits a "…failed" toast with no path back to login. |
| **A11Y-1** | Tablist / radiogroup controls have no arrow-key roving focus | `admin-ui/src/components/shared/SectionTabs.tsx:36-66`; `admin-ui/src/pages/dashboard/WindowSelector.tsx:18-44`; `admin-ui/src/pages/logsTraffic/TrafficSection.tsx:91-119` | Add `onKeyDown` (ArrowLeft/Right, Home/End) that moves focus+selection and roving `tabIndex` (active=0, rest=-1). Land it once via the shared `SegmentedControl` from UI-DUP-1. | `role="tablist"`/`role="radiogroup"` announce an interaction model the code doesn't implement — lies to keyboard/SR users. |
| **UI-DUP-1** | Three near-identical segmented controls | `SectionTabs.tsx`; `dashboard/WindowSelector.tsx`; `logsTraffic/TrafficSection.tsx:84-120` (MetricSelector) | Extract one `<SegmentedControl items value onChange ariaLabel mode='tabs'|'radio'>` owning shared styling + roving keyboard nav; convert all three call sites. | Removes ~80 lines and centralizes the A11Y-1 fix; today each copy has divergent a11y and padding. |
| **UI-DUP-2** | Empty/loading/**error** table-state patterns implemented two ways; Builds has NO error state | `admin-ui/src/pages/BuildsPage.tsx:146-232` vs `UsersPage.tsx:127-149` & `TexturesPage.tsx:204-226` | Delete BuildsPage's local `SkeletonRows` + inline empty block; use shared `TableSkeletonRows`/`TableStateRow`, and **add an `isError` branch** (AlertCircle) so a load failure is distinguishable from genuinely-empty. | `useAdminClients` errors currently render the misleading "No builds yet"; the same page family renders states inconsistently. |
| **DESIGN-1** | Build Details form grid is non-responsive (`grid-cols-2`, no breakpoint) | `admin-ui/src/pages/BuildDetailPage.tsx:311` | Change to `grid grid-cols-1 gap-4 sm:grid-cols-2`. Existing `col-span-2`/`sm:col-span-2` children clamp harmlessly in single-col mode. | Cramped on narrow viewports while the rest of the app is responsive; the Switch row's `sm:col-span-2` implies an intent the container never honors. |
| **A11Y-2** | Status/origin differentiation is hue/brightness-only against a monochrome palette | `admin-ui/src/pages/logsTraffic/StatusBadge.tsx:6-18`; `admin-ui/src/pages/UsersPage.tsx:40-48`; `index.css:62-68` | StatusBadge: keep the numeric label, add a leading icon for 4xx/5xx (mirror `BuildFilesTab` StatusBadge's AlertTriangle). Origin badges: text is already the label — drop the variant juggling for one consistent style. | Codebase's own rule is "icon+shape+text, never hue"; severity emphasis is currently brightness-only. (Note: badge text is the HTTP number, so not literally ambiguous — design-consistency, not a hard blocker.) |
| **BUNDLE-1** | No code-splitting — recharts + react-aria + all routes ship in one eager bundle | `admin-ui/src/App.tsx:1-12`; `vite.config.ts`; `components/ui/chart.tsx:2` | `React.lazy` + `Suspense` the route components (esp. `DashboardPage`, `LogsTrafficPage`, `BuildDetailPage` which pull recharts/react-aria). Optionally add a `manualChunks` entry isolating recharts. | Heavy libs load even for a quick login; highest-leverage perf win. (P1 is a priority call; technical claim is fully correct.) |
| **TEST-1** | Network domain (largest, most stateful) has the weakest test coverage | `crates/network/tests/network_api.rs`; no `#[cfg(test)]` in `friends/invites/join_requests/worlds/presence/relay/signaling.rs` | **Recommendation (needs-judgement, do it):** add DB-free unit tests for pure helpers first — `presence::effective_status` timeout boundary, `relay::classify` frame disposition, `guest_world_id→InWorld` mapping — then extend integration coverage for FoF re-validation-at-accept and the capacity race. Low risk, raises confidence in the riskiest crate. **Sequence after BUG-1/BUG-3** so new tests assert fixed behavior. | Subtle policy regressions (e.g. a `host_only` flip not revoking an outstanding FoF invite) would go uncaught today. |

### P2 — Polish / hygiene / defense-in-depth

**Backend security (defense-in-depth):**
- **SEC-4** — Open self-registration creates confirmed accounts, no email verification (`crates/yggdrasil/src/account.rs:137-143`, `crates/core/src/identity.rs:380-415`): add email-confirmation (`confirmed=false` until verified) or a registration-specific throttle/captcha + per-email/IP caps that survive a proxy; validate email format. *Why: mass account creation / username squatting.*
- **SEC-5** — Yggdrasil tokens stored/compared plaintext (`crates/core/src/auth/yggdrasil.rs:51-62,100-115,139-154`): residual-risk note (protocol echoes token verbatim). Store an HMAC of the access token and look up by that, or ensure DB-at-rest encryption + tight row access + never-logged. *Why: a read-only DB leak yields usable in-game creds for the 15-day TTL.*
- **SEC-6** — Bundles admin FS ops rely on DB existence, not `validate_slug` (`crates/bundles/src/admin.rs:118-128,134-186,251-366`): call `storage::validate_slug(&slug)` at the top of every admin handler that touches the filesystem (mirror `public.rs:27-29,63`). *Why: defense-in-depth so the FS guard doesn't depend on the DB-uniqueness invariant holding forever.*
- **SEC-7** — CSRF compare non-constant-time (`crates/core/src/auth/csrf.rs:34-44`): use `subtle::ConstantTimeEq`; optionally HMAC-bind the CSRF token to the session id. *Why: conventional token-check hardening (weak oracle today).* (low confidence)
- **SEC-8** — Security headers scoped only to `/admin` (`crates/server/src/main.rs:121-126,160-194`): apply `X-Content-Type-Options: nosniff` (+HSTS) globally at the outer router; keep strict CSP/frame admin-scoped; ensure user-uploaded file routes (textures/catalog media/bundle files) always send a correct fixed Content-Type + nosniff. *Why: prevents MIME-sniffing content confusion on uploaded bytes.*
- **SEC-9** — Bootstrap admin auto-promotes any same-named user; weak env password (`crates/admin/src/startup.rs:16-71`, `crates/core/src/config.rs:211-224`): don't auto-promote by username — fail if the username is taken (or match a known marker); enforce a strong-password check at startup. *Why: a pre-registered "admin" account could be promoted on first bootstrap.*
- **SEC-10** — TraceLayer logs full URIs; WS `?token=` fallback leaks into logs (`crates/server/src/main.rs:157`; `crates/network/src/relay.rs:46-65`, `signaling.rs:16-33`): custom `make_span` that strips the query before tracing; prefer Bearer header for WS auth. *Why: a leaked log line discloses a live session token / join ticket.*

**Backend correctness/data-integrity:**
- **BUG-4** — Relay `park()` overwrites an existing waiting peer, no role match (`crates/network/src/relay.rs:199-214`, `crates/core/src/realtime.rs:184-189`): record the parked party's role; `take()` pairs only host↔guest; on same-role second arrival return Conflict instead of insert-overwriting. *Why: leaked socket / host-host pairing on relay reconnect.*
- **BUG-5** — `update_client` orphans the previously-provisioned bundle on slug rename (`crates/catalog/src/admin.rs:193-253` → `:153-191`): capture old `bundle_id` before re-link; rename the existing bundle (slug + on-disk dir) or `delete_owned_bundle` the now-unreferenced old one. *Why: orphaned bundle row + artifacts + `builds/{oldSlug}/` with no admin cleanup surface.*
- **BUG-6** — `upsert_scan` never deletes rows for removed files; doc comment overstates ("Replace all") (`crates/bundles/src/repo.rs:276-293`): delete-then-insert in a transaction, or `DELETE ... WHERE bundle_id=$1 AND relative_path <> ALL($2)`; fix the comment. *Why: stale artifact rows accumulate after re-upload.*
- **BUG-7** — `ingest_archive` extracts over existing files without clearing build dir (`crates/bundles/src/admin.rs:188-206`, `archive.rs:36-111`): if replace is intended, clear `files_root` before extraction and pair with full artifact-row replace (BUG-6); else document additive semantics. *Why: re-upload is silently additive, contradicting the "upload a new build" model.* **(Bundle BUG-6+BUG-7 should be fixed together — same scan/disk consistency.)**
- **BUG-8** — `delete_request_logs_older_than` binds `days as f64` (`crates/core/src/request_log.rs:194-202`): bind `i64` + `now() - make_interval(days => $1)`. *Why: type-fragile clarity fix.* (low)
- **BUG-9** — Catalog `percent_decode` edge handling (`crates/catalog/src/query.rs:31-59`): use `percent-encoding` crate or change guard to `i + 3 <= bytes.len()` + add a trailing-partial-escape test. *Why: cosmetic edge bug on the optional `locale=` param.* (low)

**Backend N+1 / perf:**
- **CAT-PERF-1** (the two `PERF-1`s merge) — N+1 in catalog DTO assembly + network list endpoints (`crates/catalog/src/repo.rs:317-353`, `:209,279-314`; `crates/network/src/join_requests.rs:357-361`, `invites.rs:97-101`): batch relation reads with `WHERE client_id = ANY($1)` and fold keyword titles into a JOIN; for network lists, inline requester/host columns into the list query (cf. `users.rs` EXISTS pattern). *Why: O(rows×relations) round-trips on a 10-connection pool.* (defer until list sizes grow; low priority but worthwhile)

**Backend dedup / dead-code / quality:**
- **CORE-DUP-1** — Cookie parsing duplicated 4× (`crates/core/src/auth/mod.rs:165-172`, `auth/csrf.rs:22-29`, `request_log.rs:262-269`, `admin/src/cookies.rs:60-67`): one `pub fn cookie_value(headers, name)` in `core::auth`; collapse `request_log::session_token` to `core::auth::session_token_from_headers`.
- **SRV-DUP-2** — `resolve_ip` duplicated (`crates/server/src/reqlog.rs:82-95` vs `ratelimit.rs:95-104`): extract `fn client_ip(...)` into `server/src/ip.rs`. **Do this as part of SEC-1** (security-sensitive precedence should live in one place).
- **STORE-DUP-3** — Media fs helpers duplicated verbatim (`crates/catalog/src/store.rs:11-63` vs `crates/textures/src/store.rs:44-88`): move `revision_hex`/`write_file`/`unlink_quiet` into `core::storage`.
- **SQLX-DUP-4** — `is_unique_violation` re-implemented 4× (`friends.rs:16-18`, `invites.rs:49-51`, `catalog/admin.rs:23-25`, `identity.rs:96-98`): add `core::error::is_unique_violation`/`is_foreign_key_violation`; keep identity's constraint-name variant.
- **CFG-DUP-5** — Public-URL parsing twice (`crates/textures/src/lib.rs:44-53` vs `crates/server/src/main.rs:200-215`): one `core::config` helper returning `(origin, path)`.
- **PRES-DUP-3** — Near-identical effective-status reads (`crates/network/src/presence.rs:38-54` vs `join_requests.rs:45-61`): have `host_effective_status` delegate to `presence::effective_status_for`.
- **DEAD-1 (bundles)** — `delete_owned_bundle` dead public re-export (`crates/bundles/src/repo.rs:185-196`, `lib.rs:33`): remove it + the re-export + doc mention.
- **ARCH-2** — `set_published` interpolates a table name (`crates/catalog/src/admin.rs:356-380`): no urgent change; optionally use an enum for publishable tables and centralize the "only-fixed-literals" rule. Do NOT rewrite to `query!` (project deliberately uses runtime sqlx). (hygiene)
- **QUAL-1** — `User` `#[allow(dead_code)]` mirror hides truly-vestigial columns (`crates/core/src/models.rs:56-80`): audit `yggdrasil_validated_at`/`avatar_url`; mark mirrored-only or drop after schema review. (low; needs schema review)
- **QUAL-2** — Catalog doc triggers `doc_lazy_continuation` (`crates/catalog/src/lib.rs:7-9`): re-wrap the continuation lines (or add the documented repo-wide `-A clippy::doc_lazy_continuation`). *Why: only real clippy warnings firing today.* (trivial)
- **TEST-2** — Server router composition untested (`crates/server/src/main.rs:90-194`, no `tests/`): add `tests/router.rs` via `tower::ServiceExt::oneshot` asserting security headers present on `/admin/*`, absent on `/textures/*`, and the credentials/origin CORS matrix. **Pairs with SEC-8** (lock in the global-nosniff/admin-scoped-CSP split).

**Admin-ui bugs/quality:**
- **FORM-1** — Unhandled rejection in dialog submit (`admin-ui/src/pages/users/CreateUserDialog.tsx:85-94`, `ResetPasswordDialog.tsx:56-62`): wrap `mutateAsync` in try/catch (hook `onError` already toasts).
- **FM-1** — MoveDialog stale `selected` lets you confirm a disabled move (`admin-ui/src/features/builds/MoveDialog.tsx:66-145`): `useEffect(() => { if (open) setSelected(null); }, [open, sourcePaths])` + `disabled={pending || selected===null || disabledFor(selected)}`.
- **FM-2** — Concurrent multi-file upload races pending/invalidation (`admin-ui/src/features/builds/BuildFilesTab.tsx:243-247`, `bundles/api.ts:65-91`): upload sequentially (await each), gate busy state on the whole batch, emit one summary toast.
- **FM-3** — NewFolderDialog retains previous name on reopen (`admin-ui/src/features/builds/NewFolderDialog.tsx:30-50`): `useEffect(() => { if (open) setName(''); }, [open])` or conditionally mount.
- **STALE-1** — Build Details form doesn't re-seed after server normalization (`admin-ui/src/pages/BuildDetailPage.tsx:216,988,279-306`): reset form from `build` in `updateClient` onSuccess, or include `build.updatedAt` in the panel key.
- **MEDIA-1** — `ClientMediaSection` shows empty slots on fetch error (`admin-ui/src/features/catalog/components/ClientMediaSection.tsx:191-227`): add an `isError` branch with retry, distinct from genuinely-empty.
- **DROP-1** — Unhandled rejection in breadcrumb root DropZone (`admin-ui/src/features/builds/FileBreadcrumbs.tsx:64-85`): wrap `onDrop` in try/catch; validate `Array.isArray(payload.ids)` before use.
- **AUTH-2** — Logout `setQueryData(me,null)` wiped by `qc.clear()` then refetches (`admin-ui/src/features/auth/api.ts:44-53`): drop the dead `setQueryData`, or `qc.clear(); qc.setQueryData(authKeys.me, null);` for an immediate redirect.
- **VER-1** — `useVersions` v1 fallback casts a partial object to full catalog (`admin-ui/src/features/builds/useVersions.ts:60-67`): default all required fields (`minecraft/fabric/forge/java/recommended/version/generatedAt`) before the cast. **Touches the launcher versions.json contract — see care note.**
- **TEX-1** — "Purge missing" enabled before any scan (`admin-ui/src/pages/TexturesPage.tsx:164-171,324-346`): `disabled={purgeMissing.isPending || orphanCount===null || orphanCount===0}`, or auto-scan before opening the confirm.
- **NAV-1** — Post-create nav to `/builds/{bundleSlug}` can 404 if bundle slug ≠ client slug (`admin-ui/src/pages/BuildsPage.tsx:80-84`): navigate with the client slug used for matching (`trimmedSlug`), or match `useBuildBySlug` on both. (low; latent)
- **AUTH-PERF-1** — AuthProvider context recreated every render (`admin-ui/src/features/auth/AuthProvider.tsx:26-51`): depend on stable primitives (`session.data`, `.isLoading`, `loginMutation.isPending`, `logoutMutation.isPending`) and call `mutateAsync` inside callbacks.
- **UI-DEADCODE-1** — Unused mutation hooks (`admin-ui/src/features/catalog/api.ts:124-136,268-280,298-310,312-324`; `bundles/api.ts:196-217`): remove `useAttachMedia`/`useDeleteKeyword`/`useUpdateServer`/`useDeleteServer`/`useMoveFile`. If any are kept for future use, fix `useDeleteKeyword`/`useDeleteServer` to also invalidate `catalogKeys.clients()`.

**Admin-ui design-system consistency:**
- **UI-DUP-3** — Pagination footer duplicated (`UsersPage.tsx:192-224`, `TexturesPage.tsx:275-307`): extract `<TablePager … />`.
- **UI-DUP-4** — `StatCard`/`StatCardDisplay` duplicated + two identical `EmptyChartState` (`pages/dashboard/StatCard.tsx`, `logsTraffic/TrafficSection.tsx:420-450`): generalize `StatCard` with a `display?: string` prop; extract one shared `EmptyChartState`.
- **UI-DUP-5** — Keywords/Servers sections large near-duplicates (`pages/BuildDetailPage.tsx:528-693`, `:695-879`): extract `<AttachableEntitySection>` (or at least a shared `AddExistingPicker`).
- **A11Y-3** — `text-faint` (oklch 0.556) borderline for AA small text (`index.css:38`): bump to ~0.62 or reserve faint for ≥14px and use `text-mute` for small captions; verify with a contrast checker.
- **A11Y-4** — Processing `StatusBadge` spinner conveys live state with no `aria-live` (`features/builds/BuildFilesTab.tsx:60-86`; `BuildsPage.tsx:251-262`): add `aria-live="polite"` to the processing container. (low)
- **A11Y-5** — Redundant `font-bold` on `text-h2`; nav active lacks `aria-current` (`components/layout/AppShell.tsx:37,46-53`): drop redundant `font-bold`; add `aria-current="page"` via NavLink's `isActive`.
- **DESIGN-2** — Skeletons use `bg-accent` lighter than their surfaces (`components/ui/skeleton.tsx:9`): set fill to a recessed token (`bg-surface-2`/dedicated `--color-skeleton`); use shared Skeleton in `BuildFilesTab` instead of the bespoke `bg-surface-1` blocks.
- **DESIGN-3** — Tables lack zebra striping / sticky header (`components/ui/table.tsx`): add optional `even:bg-surface-0/40` striping + sticky header for the scrollable log table (keep subtle).
- **DESIGN-4** — Base card border too faint (`components/ui/card.tsx:10`): bump resting border to `border-edge-md` or pair faint border with `bg-surface-1`. (low)
- **DESIGN-5** — TexturesPage tabs mix icon/no-icon → misalignment (`pages/TexturesPage.tsx:80-83`): give Capes an icon or drop the Skins icon.
- **DESIGN-6** — Inconsistent dialog body spacing (`CreateUserDialog.tsx:161` vs `BuildsPage.tsx:99-119` vs `NewFolderDialog.tsx:66,76` vs `MoveDialog.tsx:91,124`): standardize on the `DialogContent` gap (wrap bodies in `flex flex-col gap-4`, drop manual `mt-*`).
- **TYPE-1** — Verbose boolean ternary + FileGrid omits download-once (`features/builds/FileList.tsx:156`; `FileGrid.tsx`): simplify to `entry.artifact !== null && !entry.isDir`; either surface download-once in grid or document the divergence (FileGrid doesn't even destructure it).
- **UI-DEAD-1** — Unused exports `ChartLegend`/`ChartLegendContent`/`ChartStyle`/`AvatarImage`; duplicate window type (`components/ui/chart.tsx:251-303,341-348`; `avatar.tsx:22-33`; `dashboard/WindowSelector.tsx:3`): drop unused chart exports (vendored shadcn — confirm no planned legend), remove `AvatarImage` if avatars stay imageless, unify `TimeseriesWindow`/`TrafficWindow` into one type.
- **COMMENT-1** — 124 `///` what-comments conflict with the why-only rule (25 files): hygiene sweep — delete descriptive `///`, keep genuine why-comments (Radix remount note `BuildDetailPage:452-459`, ring-buffer key `LiveLogsSection:161`, NONE/CUSTOM sentinels). Bundle this into each file's other edits rather than as a standalone pass.

### DROPPED / DEFERRED
- No verifier false-positives to drop — all P0/P1 items were **confirmed**. **BUG-3**'s sub-claim about the presence LATERAL was corrected (it *does* filter `ws.status='open'`); the fix is unchanged.
- **TEST-1** and **BUNDLE-1**: confirmed facts, P1 severity is a priority judgment — recommendation is **do them**, sequenced as noted.
- **ARCH-1**: confirmed defect, but **requires a product write-vs-delete decision** before implementation. Default to the write path.

---

## WAVES (parallel/sequential agent execution)

Backend crates and admin-ui touch disjoint trees → **the entire backend track and the entire admin-ui track run in parallel.** Within each track, dedup/refactor waves are sequenced before/after the bugfix waves that depend on the shared helpers.

### Wave 0 — Trivial, zero-risk, fully parallel (any agent)
- Backend: **QUAL-2** (catalog doc / clippy).
- Admin-ui: **DESIGN-5**, **A11Y-5**, **TYPE-1**, **AUTH-2**, **UI-DEADCODE-1**, **UI-DEAD-1**, **VER-1** *(care: contract — see below)*, **TEX-1**, **NAV-1**.
- All touch disjoint files; run as many in parallel as the harness allows.

### Wave 1 — P0 backend (sequential within backend; parallel with admin-ui Wave 1)
- **SEC-1 + SEC-3 + SRV-DUP-2** together (single agent) → creates `crates/server/src/ip.rs`, rewires `ratelimit.rs` + `reqlog.rs` + `main.rs`. *Security-sensitive IP precedence in one place.*
- **BUG-1** (independent files: `friends.rs`, `presence.rs`) — can run parallel to the SEC-1 agent.

### Wave 1 (admin-ui, parallel to backend Wave 1)
- **AUTH-1** (queryClient/client/auth api) + **AUTH-PERF-1** (AuthProvider) — same auth area, one agent.
- **UI-DUP-1 + A11Y-1** together (creates `SegmentedControl`, converts 3 call sites).
- **FORM-1**, **FM-1**, **FM-3**, **DROP-1**, **MEDIA-1**, **STALE-1**, **FM-2** — file-manager/dialog bugs, mostly disjoint files; can fan out 2-3 agents.

### Wave 2 — P1/P2 backend correctness (after Wave 1; some sequential)
- **BUG-3** (worlds.rs — factor shared close-cleanup helper). Independent file.
- **BUG-2** ⚠️ — prefer fix (a) join-by-user-id; touches `textures/handlers.rs` + `core/identity.rs`. **Migration-adjacent** (textures schema invariant) — extra care; do alone.
- **BUG-4** (relay.rs + realtime.rs), **BUG-5** (catalog/admin.rs), **BUG-6 + BUG-7 together** (bundles repo/admin/archive), **BUG-8**, **BUG-9** — disjoint crates, fan out.
- **SEC-2**, **SEC-6**, **SEC-8 (+ TEST-2)**, **SEC-9**, **SEC-10**, **SEC-7** — security hardening, mostly disjoint (`main.rs`/`infra.rs`/`bundles/admin.rs`/`startup.rs`/`csrf.rs`). SEC-2/SEC-8/SEC-10 all touch `main.rs` → **one agent for the main.rs-touching set** to avoid edit collisions.
- **SEC-4** (registration controls) — yggdrasil/account.rs + identity.rs. May touch migrations if `confirmed=false` default chosen — **migration care**.
- **ARCH-1** ⚠️ — only after the write-vs-delete decision; if "write," moves code into `core` + wires `core` callers (touches hot paths). Do alone.

### Wave 2 (admin-ui, parallel)
- **UI-DUP-2** (BuildsPage table states), **DESIGN-1** (BuildDetail grid), **A11Y-2** (StatusBadge), **BUNDLE-1** (App.tsx lazy + vite manualChunks).
- **UI-DUP-3/4/5**, **DESIGN-2/3/4/6**, **A11Y-3/4** — design-system consolidation; UI-DUP-4/5 and DESIGN-* touch shared `components/ui/*` so coordinate (one agent per shared component file).

### Wave 3 — Dedup/cleanup backend (after Wave 2 bugfixes land)
- **CORE-DUP-1**, **STORE-DUP-3**, **SQLX-DUP-4**, **CFG-DUP-5**, **PRES-DUP-3**, **DEAD-1 (bundles)**, **ARCH-2**, **QUAL-1**, **CAT-PERF-1** (defer-OK).
- **TEST-1** (network unit + integration tests) — **last**, so tests assert post-fix behavior.
- **COMMENT-1** — fold into whichever agent last touches each file.

### EXTRA-CARE callouts
- **Migrations / schema:** BUG-2 (textures denormalized `profile_uuid` invariant), SEC-4 (`confirmed` default), QUAL-1 (column drops). Any migration → bump `migrations/`, keep `sqlx::migrate!` checksums consistent, and remember the rust-embed SPA compile-time embedding gotcha.
- **Launcher manifest / contract:** **VER-1** touches the launcher `versions.json` shape — keep the public contract backward-compatible (default missing fields, never tighten). **BUG-6/BUG-7** change bundle manifest regen semantics — verify the served manifest URL stays correct for the launcher.

---

## PER-WAVE VERIFICATION

**Backend (run from repo root; Postgres on :5433):**
```
$env:DATABASE_URL = "postgres://postgres:postgres@localhost:5433/loontail_test"
$env:YGGDRASIL_PUBLIC_URL = "/api/yggdrasil"
cargo build --workspace
cargo test  --workspace
cargo clippy --workspace --all-targets -- -A clippy::doc_lazy_continuation
cargo fmt --all -- --check
```
- After Wave 1 (SEC-1): add a unit test that, with a peer present **and** `TRUSTED_PROXY` set, the limiter buckets by the XFF hop, not the peer.
- After BUG-1: add a network integration test for a credential-only friend (register, no bootstrap) hitting `GET /friends` / requests / presence → 200, not 500.
- After BUG-3: assert a `PATCH status=closed` closes relay_sessions + resets host presence + zeroes current_players (mirror the `close()` assertions).
- After SEC-8 + TEST-2: `tests/router.rs` asserts `nosniff` present globally, CSP/frame present on `/admin/*` and absent on `/textures/*`, and the CORS credentials/origin matrix.

**Admin-ui (run from `admin-ui/`):**
```
npm run build
npm test
npm run lint   # biome — do not introduce ESLint
```
- After AUTH-1: a test that a 401 on a non-login query resets `authKeys.me` → RequireAuth redirects.
- After UI-DUP-1/A11Y-1: keyboard test (arrow/Home/End move selection; single tab stop) on the shared `SegmentedControl`.
- After BUNDLE-1: confirm `npm run build` emits separate recharts/route chunks (check the rollup output / dist chunk listing).

**Live re-embed (when manually exercising the running server):**
```
# After admin-ui changes, rebuild the SPA, then force the embed to refresh:
(cd admin-ui && npm run build)
# touch the embedding source so rust-embed re-includes the new dist:
[IO.File]::SetLastWriteTime("crates/admin/src/spa.rs", (Get-Date))   # PowerShell
cargo build -p loontail-server   # re-embed
# also `touch crates/core/db.rs` if migrations changed, per the known compile-time-embed gotcha
```

**Final gate (all waves merged):** full backend `cargo test --workspace` + `clippy` + `fmt`; admin-ui `npm run build && npm test`; one manual smoke of login → builds → file ops → textures with the rebuilt+re-embedded server (`YGGDRASIL_PUBLIC_URL=/api/yggdrasil`). Baseline to beat: 207 backend + 46 admin-ui green (per project memory) — net should rise with the new BUG-1/BUG-3/AUTH-1 tests.

Relevant paths are inline above; the two new files to create are `E:\workspace\elixir\loontail-minecraft-network-service\crates\server\src\ip.rs` (SEC-1/SRV-DUP-2) and `E:\workspace\elixir\loontail-minecraft-network-service\crates\server\tests\router.rs` (TEST-2/SEC-8).