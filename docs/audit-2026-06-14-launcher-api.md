# Loontail Launcher API — Consolidated Audit (2026-06-14)

Lead-auditor synthesis of 19 parallel reports over the backend (`loontail-minecraft-network-service`, the future `loontail-launcher-api`), its admin SPA, the Electron launcher, and the in-game network agent/mod. Scope: security, architecture/quality, the integration surface for unifying auth onto a single Yggdrasil access token (and removing `api_tokens`), observability inputs for the planned request-log + traffic dashboard, and the repo rename.

---

## 1. Executive Summary

**State of the API.** The backend is a clean, acyclic 9-crate Cargo workspace (a pure `yggdrasil-protocol` leaf, a `core` kernel, six domain crates, one `server` binary). Cross-cutting fundamentals are strong: every query uses runtime sqlx with bound parameters (zero `query!` macros workspace-wide — no SQL injection found), the RSA-SHA1 texture signing is byte-for-byte Mojang-compatible and golden-vector-pinned, all token randomness is CSPRNG-backed, opaque tokens are 256-bit and SHA-256-hashed at rest, passwords use Argon2id, `User.password_hash` never reaches a serializer, `AppError` maps DB/internal failures to a generic 500, and the four auth namespaces are structurally non-cross-acceptable (one table per extractor). Error discipline is high: almost every `unwrap()/expect()` is test- or build-only.

**Top risks.**
1. **Unauthenticated path traversal** via the bundle `{slug}` on the two public manifest/file endpoints — the only fully-public byte-serving surface, with a self-referential `starts_with` guard. (3 reporters independently flagged this; highest impact.)
2. **Block/demote does not revoke live sessions** — the network `AuthUser` extractor never re-checks `users.blocked`, so blocking a user leaves all live network sessions valid until TTL.
3. **No rate limiting / brute-force lockout** on any of the three credential endpoints, and the server can't even see client IPs (`into_make_service`, no `ConnectInfo`). Compounded by a login user-enumeration timing oracle.
4. **CORS defaults to `*` with `Any/Any`** on a cookie-authed admin surface — one `allow_credentials(true)` line away from a credentialed-wildcard hole.
5. **No security headers at all** (no HSTS/CSP/X-Frame-Options/nosniff) on the cookie-authenticated admin SPA.
6. **In-memory rendezvous forces single-process** — horizontal scaling silently breaks join/relay/presence; plus no runtime reaper for stale presence and a leaking relay player-slot counter.
7. **Memory-exhaustion uploads** — bundle archive route disables the body limit then buffers the whole payload in RAM; single-file upload has no cap.
8. **`realtime.rs` `lock().unwrap()`** panics on poison where the sibling `join_sessions.rs` already recovers — the lone real request-path panic source.
9. **Dead analytics pipeline** — `record_event` has zero production callers, so `user_events` is empty in prod and the dashboard timeseries always renders empty; the redesign's traffic dashboard is greenfield.
10. **Integration**: moving to one Yggdrasil token requires a new public registration endpoint, promoting `validate_yggdrasil`/`YggdrasilUser` to the universal Bearer authenticator, a launcher→agent token handoff that doesn't exist yet, and re-gating today's fully-public bundle reads.

The acyclic layout means the auth unification is a localized `core` change (one new extractor) plus mechanical extractor swaps in domains. None of the findings block the redesign; several must land alongside it (block-recheck, CORS/credentials discipline, the `Unauthorized` message neutralization).

---

## 2. Security Findings

Sorted critical→info, de-duplicated across reporters. File paths are relative to the repo root (`crates/...`).

### HIGH

#### S1. Unauthenticated path traversal via bundle `{slug}` on public manifest & file endpoints
*(Reported by SQLi/input-handling, Authorization/IDOR, and crate-boundaries reporters.)*

`GET /api/bundle-registry/builds/{slug}/manifest` (`public::get_manifest`) and `GET /bundle-registry/builds/{slug}/files/{*path}` (`public::serve_file`) feed the raw `{slug}` path segment straight into the filesystem via `manifest_path`/`files_path` → `build_path` → `builds_root().join(slug)`, with **no** `normalize_relative_path` and **no** `require_by_slug` DB existence check. axum percent-decodes `Path` params after routing, so `/builds/..%2F..%2Fsomedir/manifest` yields `slug = "../../somedir"`, escaping `{storage_root}/builds`. The defense `if !target.starts_with(&root)` is tautological because `root = files_path(storage_root, &slug)` is itself derived from the malicious slug. The `{*path}` tail is correctly normalized; the slug is the hole. Every admin handler gates the slug through `require_by_slug` first — the public handlers do not. This is the only fully-public byte-serving surface.

- **Files:** `crates/bundles/src/public.rs:19-78`, `crates/bundles/src/storage.rs:16-28`, `crates/bundles/src/lib.rs`
- **Fix:** Validate the slug as a single segment (reject `/`, `\`, `..` after percent-decode) **and** call `repo::find_by_slug`/`require_by_slug` first, 404'ing absent slugs before any FS join. Recompute `root` from the fixed `builds_root(storage_root)` (not the slug-derived path) and canonicalize `target` before `starts_with`. Add a traversal regression test (current only traversal test covers zip-slip, not the slug).

#### S2. Network `AuthUser` never re-checks `users.blocked` — blocking leaves live sessions valid
*(Auth-namespaces; also surfaced by network-domain recon as a security upgrade after the switch.)*

`user_from_token` (`crates/core/src/auth/mod.rs:62-79`) joins `network_sessions`→`users` with only `revoked_at IS NULL AND expires_at > now()`. There is **no** `u.blocked = false` predicate, unlike `validate_admin_session` (`admin.rs:64-66`) and `validate_yggdrasil` (`yggdrasil.rs:123` checks `blocked || !confirmed`). The admin block handler (`crates/admin/src/users.rs:127-137`) only flips `users.blocked` and never revokes sessions. A blocked user keeps full friends/presence/world/join/relay access until natural expiry (default 86,400s). Amplified once one Yggdrasil token authorizes the whole API.

- **Files:** `crates/core/src/auth/mod.rs:62-79`, `crates/admin/src/users.rs:127-137`
- **Fix:** Add `AND u.blocked = false` to `user_from_token`; have the block handler also call `revoke_all_network_sessions_for_user` + `invalidate_all_yggdrasil_for_user`. (The Yggdrasil extractor already enforces blocked/confirmed — adopting it for network is itself the fix.)

#### S3. No rate limiting / brute-force lockout on any credential endpoint; server can't see client IPs
*(Web/transport; reinforced by kernel — missing 429 variant.)*

Three unauthenticated endpoints run straight into Argon2id with zero throttling: admin login `POST /admin/auth/login` (`crates/admin/src/auth.rs:22-46`), Yggdrasil `POST /authserver/authenticate` (`crates/yggdrasil/src/lib.rs:127-153`), and `POST /authserver/refresh`. No `tower_governor`/limiter dependency exists, and `main.rs:66-69` serves via `into_make_service` (not `_with_connect_info`), so no per-client key is even available. The single-token redesign makes credential-guessing the highest-value attack. The design spec explicitly defers rate limiting "beyond a basic guard" — that guard does not yet exist.

- **Files:** `crates/admin/src/auth.rs`, `crates/yggdrasil/src/lib.rs`, `crates/core/src/identity.rs`, `crates/server/src/main.rs`, `crates/server/Cargo.toml`
- **Fix:** Per-account + per-IP attempt counter with backoff/lockout, plus a global `tower_governor` limiter on auth routes. Switch to `into_make_service_with_connect_info::<SocketAddr>` with a trusted-proxy `X-Forwarded-For` parser. Add an `AppError::TooManyRequests` (429) variant (`crates/core/src/error.rs`).

#### S4. CORS defaults to `*` with `Any/Any` on a cookie-authed surface — one line from credentialed-wildcard
*(Web/transport; cross-referenced by authorization reporter on CSRF interaction.)*

`build_cors()` (`crates/server/src/main.rs:158-169`) always sets `allow_methods(Any)` + `allow_headers(Any)`; with `CORS_ALLOWED_ORIGINS` unset, `config.rs:80-85` defaults to `['*']` → `allow_origin(Any)`. No `allow_credentials(true)` today (so the admin cookie isn't reflected cross-origin), but the planned "admin cookie authorizes the whole API" makes adding that one line turn this into a credentialed wildcard that tower-http reflects per-origin, defeating the SameSite/CSRF posture. `allow_headers(Any)` already makes `x-csrf-token` cross-origin-settable. Fail-open default is unsafe for a fresh deploy.

- **Files:** `crates/server/src/main.rs:158-169`, `crates/core/src/config.rs:80-85`, `crates/admin/src/cookies.rs`, `crates/core/src/auth/csrf.rs`
- **Fix:** Default `CORS_ALLOWED_ORIGINS` to a closed list (fail closed). Never combine `allow_origin(Any)` with credentials; use an explicit origin allowlist + explicit header/method allowlists (`authorization`, `content-type`, `x-csrf-token`). Add a startup assertion rejecting credentials-while-Any.

### MEDIUM

#### S5. No HTTP security headers on the cookie-authenticated admin SPA
No `Strict-Transport-Security`, `X-Content-Type-Options: nosniff`, `X-Frame-Options`/CSP `frame-ancestors`, or `Referrer-Policy` anywhere (grep returns nothing). The admin SPA performs privileged mutations under an httpOnly cookie yet is clickjackable, sniffable, and (without HSTS) downgrade-strippable; a CSP would also harden against XSS reading the JS-readable `loontail_csrf` cookie.
- **Files:** `crates/server/src/main.rs`, `crates/admin/src/spa.rs`, `crates/admin/src/cookies.rs`
- **Fix:** Add a `SetResponseHeaderLayer` (or small middleware) applying HSTS, nosniff, `X-Frame-Options: DENY` + CSP `frame-ancestors 'none'`, `Referrer-Policy: no-referrer`, and a restrictive CSP scoped to `/admin` (don't break Yggdrasil/bundle/texture file responses).

#### S6. Bundle uploads disable the body limit and buffer the whole payload in memory; single-file upload uncapped
*(SQLi/input-handling and panics/error-discipline reporters.)*

The archive route is `post(admin::upload_archive).layer(DefaultBodyLimit::disable())` (`crates/bundles/src/lib.rs:53`); the handler's comment claims streaming but actually does `field.bytes().await...to_vec()` (`admin.rs:141-145`) before `fs::write`, fully resident in RAM (the 10 GiB/100k-entry guards apply to extracted size, not the request body). `upload_file` (`admin.rs:229-233`) reads `field.bytes()` fully with **no** size cap. Admin-gated, but the redesign widens blast radius if an admin token leaks. Textures (`handlers.rs:281-289`) also buffer-then-check.
- **Files:** `crates/bundles/src/admin.rs`, `crates/bundles/src/lib.rs`, `crates/bundles/src/archive.rs`, `crates/textures/src/handlers.rs`
- **Fix:** Stream each field via `field.chunk().await` into the temp file with a running byte cap; give `upload_file` an explicit cap + sane `DefaultBodyLimit` mirroring the textures `MAX_UPLOAD_BYTES` pattern.

#### S7. Timing-unsafe comparison of plaintext Yggdrasil tokens (and CSRF double-submit)
`validate_yggdrasil`/`refresh_yggdrasil` resolve the access token via `WHERE access_token = $1` then compare `client_token` with Rust `!=` (short-circuits per byte; `crates/core/src/auth/yggdrasil.rs:100-154`). `verify_csrf` does `cookie != header` on the raw token (`crates/core/src/auth/csrf.rs:40`). This is the redesign's main hot path.
- **Fix:** Constant-time compare (`subtle::ConstantTimeEq`/`ring`) for `client_token` and CSRF; prefer indexing the access token by SHA-256 hash (the Mojang echo can stay plaintext on the echo endpoints only — see S15).

#### S8. RSA private key persisted with default umask only (no 0600)
`load_or_generate_key` writes the PKCS#8 PEM with plain `std::fs::write` and `create_dir_all` — no `mode(0o600)`/ACL tightening anywhere (`crates/yggdrasil/src/crypto.rs:100,114,125`). The long-lived RSA-4096 identity at `data/yggdrasil/keys/active.key.pem` may land world/group-readable on a shared host; anyone who reads it forges signed GameProfiles. `.gitignore` excludes it from VCS, but at-rest protection is left to ambient umask.
- **Fix:** After write, `#[cfg(unix)]` set `0o600` on the key and `0o700` on the dir (use `OpenOptions.mode(0o600)` on create so it's never momentarily world-readable); restrict the Windows ACL; verify/repair on the load path.

#### S9. Admin login is CSRF-able / session-fixation-adjacent (SameSite=Lax, no Origin/Referer check)
Admin session+CSRF cookies are `SameSite=Lax` (`crates/admin/src/cookies.rs:14`); the login handler does no Origin/Referer check, enabling login-CSRF (silently logging a victim into an attacker account). No session-rotation on privilege change.
- **Files:** `crates/admin/src/cookies.rs`, `crates/admin/src/auth.rs`
- **Fix:** `SameSite=Strict` for the same-origin SPA session, Origin/Referer allowlist on login, consider `__Host-` cookie prefix.

#### S10. Privilege downgrade / block does not revoke existing tokens
`PATCH /admin/users/{id}` can set `is_admin=false` but (unlike `reset_password`/`revoke_tokens`) never calls `revoke_all_admin_sessions_for_user` (`crates/admin/src/users.rs:87-108`). `validate_admin_session` re-checks `is_admin` per request (so AdminUser routes lock out), but the user's `network_sessions`/`yggdrasil_tokens` survive — fatal under the single-token model where `is_admin` carried by that token authorizes the whole API.
- **Fix:** Revoke admin sessions on any `is_admin` transition; under the redesign, demotion/block must invalidate the user's Yggdrasil tokens too.

#### S11. No admin self-protection: admin can demote/block/delete self or the last admin
`crates/admin/src/users.rs` mutators never read `_admin.id()` and enforce no last-admin invariant, so an admin can lock out the entire admin surface (recoverable only via `ADMIN_BOOTSTRAP_PASSWORD`). Not escalation, but a denial-of-administration that becomes worse when `is_admin` authorizes the whole API.
- **Fix:** Self-target guard on destructive/role-changing mutations + a "cannot remove the last admin" count check.

#### S12. Bearer access tokens accepted in `?token=` on WebSocket endpoints while TraceLayer logs the full URI
`/signaling?token=` (`signaling.rs:24-34`) and `/relay/{id}?token=` (`relay.rs:56-65`) accept the bearer in the query; `TraceLayer::new_for_http()` (`main.rs:111`) logs `http.target`/uri including query, with `loontail_network=debug` default — tokens land verbatim in app/proxy/browser-history logs. Under the redesign the leaked value is the master credential.
- **Fix:** Require the `Authorization` header for WS upgrades (clients are the agent/mod, not browsers); if a query token must stay, custom `make_span_with` records only `uri().path()`; lower the prod log level.

#### S13. realtime.rs `lock().unwrap()` panics on poison where sibling already recovers
*(Panics/error-discipline reporter — the lone real request-path panic class.)*

7 `std::sync::Mutex.lock().unwrap()` calls on the hot WS path (`crates/core/src/realtime.rs:111,118,130,140,145,179,184`) panic on poison; once poisoned, **every** signaling/relay op panics and cascades the outage. The sibling `crates/yggdrasil/src/join_sessions.rs:49,59` already recovers identical ephemeral data with `unwrap_or_else(|e| e.into_inner())`.
- **Fix:** Use `unwrap_or_else(|e| e.into_inner())` at all 7 sites (or switch the module to `parking_lot::Mutex`).

### LOW

- **S14. LIKE search patterns don't escape `%`/`_` wildcards.** Parameterized (no SQLi) but unescaped: network friend search (`crates/network/src/users.rs:174`) and admin user search (`crates/core/src/identity.rs:391`). `%` matches all; `_` any char — wildcard injection / expensive scan. The admin path also uses a fragile `$1 = '%%'` "no-filter" sentinel. **Fix:** escape `\ % _` and add `ESCAPE '\'`; replace the sentinel with an explicit `Option` branch.
- **S15. Single-token model concentrates reliance on plaintext-at-rest tokens.** Yggdrasil tokens are stored plaintext by Mojang necessity; making them the universal credential means a DB read yields directly-usable whole-API tokens. **Fix:** store a SHA-256 lookup column for non-Mojang API paths, keep plaintext only for the Mojang echo endpoints; ensure DB-at-rest encryption + tight column grants.
- **S16. RSA signing uses the non-blinded path** (`sign()` not `sign_with_rng()`; `crypto.rs:86,90`) — theoretical Marvin-class timing side-channel; one-line fix (update `signing_is_deterministic` test, golden vector unchanged).
- **S17. Argon2id uses library defaults with no config knob** (`identity.rs:24,39`) — OWASP-floor today but can't be raised without a recompile and a future `Argon2::default()` change would silently weaken. **Fix:** explicit pinned `Params` from config + a test asserting m/t/p.
- **S18. User enumeration / timing oracle on login** — `authenticate_password` short-circuits before Argon2 when the user is absent/`password_hash` NULL (`identity.rs:239-272`). **Fix:** dummy Argon2id verify against a fixed hash on the absent path.
- **S19. Inconsistent auth-failure status codes** — yggdrasil returns 403 for unauthenticated requests where 401 is correct (`yggdrasil.rs`). **Fix:** 401 for missing/invalid creds, 403 only for authenticated-but-unauthorized.
- **S20. yggdrasil_tokens revocation is delete-only and non-atomic** — `refresh_yggdrasil` DELETE-then-INSERT outside a tx can double-issue; `invalidate_all_yggdrasil_for_user` is an N+1 loop. **Fix:** wrap refresh in a tx with conditional `DELETE ... RETURNING`; replace the loop with one `DELETE WHERE user_id = $1`.
- **S21. Bootstrap admin password never rotated/cleared; auto-promotes same-named user.** `ensure_bootstrap_admin` keeps the env secret resident, the predictable `admin` username, and silently promotes a pre-existing same-named account (`crates/admin/src/startup.rs:16-71`). **Fix:** one-time use + must-reset-on-first-login, non-guessable username, don't auto-promote.

### INFO (positive / verified-safe)
- **S22.** No SQL injection: every query is bound-parameter sqlx; the only `format!`-into-SQL sites concatenate compile-time `&'static str` allowlists (network invites/friends, catalog COLS, analytics bucket/interval). Zip-slip, the relative-path normalizer, and the rust-embed SPA are all properly defended.
- **S23.** All RNG is CSPRNG-backed (`thread_rng`/`OsRng`), opaque tokens are 256-bit SHA-256-at-rest, texture signing is golden-vector + openssl-pinned and Mojang-compatible. No MD5/SHA1 misuse (the only SHA-1 is the mandated texture signature).
- **S24.** `api_tokens` scopes are stored/edited/surfaced but **never enforced** (`authorize_public_read` treats any valid token as public read) — security theater that the planned removal correctly eliminates; ensure reads don't degrade to unconditional public when `api_tokens` is dropped.
- **S25.** `AppError` does not leak internals (Database/Internal → generic 500, logged not returned). Network IDOR surface (friends/worlds/joins/invites/relay) is correctly ownership-checked; no mass-assignment in DTOs (explicit COALESCE-per-column). `POST /users/bootstrap` trusts client-supplied `minecraft_uuid` with no proof-of-ownership — an auth-strength gap the redesign closes by replacing it with token-authenticated identity.

---

## 3. Architecture & Code-Quality Findings

Sorted high→info, de-duplicated.

### HIGH

#### A1. Dead analytics pipeline — `user_events` is never written in production
*(Crate-boundaries, dead-code, and observability reporters all flag this.)*

`admin::analytics::record_event` (`crates/admin/src/analytics.rs:138-151`) is the only writer to `user_events` and has **zero** production callers (only `admin_api.rs` tests). The dependency graph forbids domains from depending on `admin`, so the documented "domains emit events" can't happen. `POST /users/bootstrap` bumps an in-memory counter but writes no row, so `/admin/analytics/timeseries` and the Dashboard "Client bootstraps" graph render the empty state forever.
- **Files:** `crates/admin/src/analytics.rs`, `crates/network/src/users.rs`, `migrations/0008_admin_analytics.sql`
- **Fix:** Move the event-write primitive **down into `core`** (`core::events`) so any domain can emit; keep only read/aggregate handlers in `admin`; wire bootstrap/user-creation to emit. This is a prerequisite for the redesign's request-log dashboard (which needs a core-level write seam usable from global middleware).

#### A2. Duplicated, divergent `api_token` verification across crates
`catalog::apitoken::verify_api_token` (bool, absent-token=allowed; the only wired path) and `admin::tokens::verify_api_token` (returns row id; test-only dead) independently hash the same table with different semantics. A god-seam waiting to drift.
- **Files:** `crates/catalog/src/apitoken.rs:30-61`, `crates/admin/src/tokens.rs:146-155`, `crates/admin/src/lib.rs:26`
- **Fix:** Under the redesign, delete **both** plus `authorize_public_read` and re-gate on the unified Yggdrasil extractor.

#### A3. Dead reconciliation function diverges from the live bootstrap path
`core::identity::find_or_create_from_bootstrap` (+ `update_bootstrap_row`, `bind_minecraft_uuid`, `find_by_minecraft_uuid`, `find_by_profile_uuid`; `crates/core/src/identity.rs:86-231`) implements the documented 3-branch reconciliation (bind onto an existing credential-first account by `profile_uuid`) but is **test-only dead**. The live `POST /users/bootstrap` (`crates/network/src/users.rs:60-83`) does a divergent inline `INSERT ... ON CONFLICT (minecraft_uuid)` that never computes `profile_uuid`, so a credential user who later logs in via the mod gets a **second** row — the exact case the dead code exists to prevent. Two sources of truth for one invariant.
- **Fix:** Have the bootstrap handler call `find_or_create_from_bootstrap` (preferred — gives bootstrapped accounts a consistent `profile_uuid` and an upgrade path to credential login), or delete the dead function set.

#### A4. admin-ui has no Biome (or any) linter/formatter despite being a "Biome stack"
No `biome.json`, no `@biomejs/biome` devDependency, no `lint`/`format`/`check` script; the only static gate is `tsc`. The Biome conventions this audit checks against are not tooling-enforced.
- **File:** `admin-ui/package.json`
- **Fix:** Add `@biomejs/biome` + `biome.json` (extend the launcher's config) + `lint`/`format` scripts; wire `biome check` into CI.

### MEDIUM

- **A5. Auth abstractions hard-wired to the four-token model.** Four parallel extractors (`AuthUser`/`YggdrasilUser`/`AdminUser` + catalog api-token gate), each hard-coded per domain; `AppError::Unauthorized` ships a hardcoded `"missing or invalid network session token"` message returned API-wide (already wrong for catalog/admin/yggdrasil 401s; fully wrong post-redesign). The acyclic layout means unification is a localized `core` change. **Fix:** one `Authenticated`/`Account` core extractor + an `is_admin`-gated variant; neutralize the `Unauthorized` message now (`crates/core/src/error.rs:49-53`).
- **A6. `record_event`/event-write seam misplaced in `admin`** (see A1) — the write side of analytics lives in a crate it can never be invoked from.
- **A7. N+1 delete loop revoking a user's Yggdrasil tokens** (`crates/admin/src/users.rs:186-196`) — one `DELETE WHERE user_id = $1` does it; inconsistent with the session-revocation siblings. **Fix:** add a single-statement `invalidate_all_yggdrasil_for_user` in `core::auth::yggdrasil`.
- **A8. Token-hash + Bearer-extraction helpers duplicated 3×.** `catalog::apitoken::bearer`/`hash_api_token` copy `core::auth::bearer_token_from_headers`/`hash_token`; `yggdrasil::random_64_hex` duplicates `core::auth::generate_token`. **Fix:** depend on `core`; the catalog copies vanish with `api_tokens` removal, but the yggdrasil/`generate_token` dup should be merged regardless.
- **A9. Config performs zero validation; silent fallbacks.** `parse_env` swallows parse errors (`config.rs:163-171`); an empty `CORS_ALLOWED_ORIGINS` silently blocks all browsers; pool size hardcoded at 10 (`db.rs:10-13`). **Fix:** `Config::validate()` warning on unparseable env, rejecting nonsensical values; make pool size and the CORS-empty behavior explicit.
- **A10. No request-logging seam in the kernel** — only ephemeral `TraceLayer` stdout + in-process counters + the (unwritten) `user_events`. `AppState` has no request-log sink. The single biggest kernel gap vs the redesign. **Fix:** add a capture middleware + `request_logs` table surfaced through `AppState` (see §5).

### LOW / INFO

- **A11.** Rust-style `///` decorative comments in 9 admin-ui TS files (non-idiomatic; Biome would treat them as `//`); violates the repo no-decorative-comments policy. **Fix:** `//`/JSDoc or delete name-restating ones.
- **A12.** Hardcoded admin search `PAGE_SIZE = 25` bypasses the existing `search_*` config knobs (`identity.rs:388`); reconcile with the `search_max_results=20` sibling.
- **A13.** Repeated unreachable `Duration::from_std(...).unwrap_or(days(N))` magic-number TTL fallbacks in three session issuers (`admin.rs:36`, `yggdrasil.rs:49`, `sessions.rs:23`). **Fix:** drop the dead fallbacks; config is the single TTL source of truth.
- **A14.** `core` carries network-domain types (`ServerEvent`, `JoinTicketDto`) — a mild god-module symptom (the in-memory hub lives in shared `AppState`); not a cycle. Optional: make `SignalingHub` generic over the payload.
- **A15.** `migrations/0008` mixes admin/api-token/analytics/presence concerns in one file and pre-bakes the `api_tokens` table the redesign removes (drop via a new forward migration, not by editing 0008).
- **A16.** Vestigial dead data: `users.yggdrasil_validated_at` (written/read nowhere; `models.rs:79`), `User` `SELECT *` + `#[allow(dead_code)]` pulls `password_hash` over the wire on hot paths (safe only because `User` is never `Serialize` — keep that invariant). Several over-`pub` items (`assign_or_reconcile_profile_uuid`, catalog `verify_api_token` re-export, `textures::Kind`).
- **A17. Fire-and-forget `let _ =` on presence/realtime broadcasts** silently drops DB-failure errors (`signaling.rs:46-80`, `presence.rs:208`, `friends.rs:287-288`, `relay.rs:181-192`) — invisible to the planned dashboard. **Fix:** log on `Err`. (The relay oneshot/Close `let _ =` are infallible-by-design; leave them.)
- **A18. Yggdrasil error mapper discards the specific BadRequest/Conflict message** (`yggdrasil/src/error.rs:74-90`) — generic 400 hides the actionable reason; deliberate Mojang-envelope flattening, document or forward via the unused `cause` field.
- **A19.** `sqlx` `macros` feature is enabled workspace-wide but only `FromRow`/`migrate!` use it; the no-`query!`-macro rule is intact (grep-clean) but enforced by convention — a CI grep would make it mechanical.
- **A20.** DTO serde camelCase is verified clean and faithfully mirrored by admin-ui TS types (the only `UPPERCASE` exception is the Mojang texture-property contract).
- **A21.** Analytics window options diverge: backend `resolve_window` accepts 90d but the UI `WindowSelector` only offers 24h/7d/30d (dead backend capability).

---

## 4. Integration Surface — Single Yggdrasil Token & `api_tokens` Removal

**Target model:** one Yggdrasil access token (Bearer) on every launcher **and** agent request; `is_admin` (carried by the resolved user) authorizes the admin API; the browser admin SPA keeps an httpOnly cookie session minted from the same credentials. `network_sessions` and `api_tokens` are deleted. The Mojang in-game handshake (`/join`, `/hasJoined`) is explicitly **carved out** — it must keep body/query token transport for vanilla-client compatibility.

**Foundation that already exists:** `validate_yggdrasil`/`YggdrasilUser` (`crates/core/src/auth/yggdrasil.rs:95,223`) already validates a Bearer access token, returns the full `User` (incl. `is_admin`/`blocked`/`confirmed`), and is already used by textures PUT/DELETE. Promoting it to the universal authenticator is mostly mechanical because the workspace is acyclic and all extractors already live in `core`.

### 4.1 Launcher (Electron — `loontail-launcher`)

Today the launcher uses **two** client layers: a generic Strapi client (`src/main/infra/http.ts`) that attaches the static `API_TOKEN` only for `auth:'apiToken'` (used by exactly two callers — catalog `clientsApi.ts:103`, bundles `api.ts:48`), and `@loontail/yggdrasil-client` which already does password login + validate/refresh/invalidate + textures with the Yggdrasil token (encrypted in `safeStorage`/sqlite, rotation already wired). `src/shared/contracts/auth.ts:88-90` explicitly documents "the Yggdrasil access token is NOT a valid bearer for the Strapi content API" — the redesign inverts exactly this.

| Method | Endpoint | Auth today |
|---|---|---|
| GET | `/api/clients?populate[...]&locale` | static `API_TOKEN` (Bearer) via http.ts |
| GET | `/api/bundle-registry/builds/{slug}/manifest` | static `API_TOKEN` (Bearer) via http.ts |
| GET | `/bundle-registry/builds/{slug}/files/{path}` | **none** (anonymous raw GET) |
| GET | media `/uploads`, texture URLs | **none** (anonymous) |
| POST | `/authserver/{authenticate,refresh,validate,invalidate}` | Yggdrasil token (in body) — already wired |
| PUT/DELETE | `/textures/{skin,cape}` | Yggdrasil access token (Bearer) — already wired |

**Required changes:**
1. **Remove `API_TOKEN`** from `src/main/config.ts:22`, `electron.vite.config.ts:30`, `tests/setup/env.ts:2`; keep `API_URL`; keep `YGGDRASIL_API_ROOT` defaulting to `${API_URL}/api/yggdrasil`.
2. **`http.ts`**: replace `AuthMode='apiToken'|'none'` + `resolveBearer` with an `auth:'yggdrasil'` mode that reads the live access token from `getStoredAuth()` and sets `Authorization: Bearer`. Update the contradicting comments at `http.ts:11-12` and `auth.ts:88-90`.
3. **Flip the two callers** (`clientsApi.ts:103`, `bundle/api.ts:48`) to the new mode — a 2-line change once http.ts supports it.
4. **Add refresh-and-retry on 401/403** to the http.ts path (reuse `yggdrasilAuth.verifySession`). Today only the YggdrasilClient path refreshes; the static token never expired — this is the most likely regression source.
5. **Decide logged-out catalog browsing** (gate behind a session vs keep public read and attach bearer only when present).
6. **Bundle file download (`download.ts`) + media cache (`mediaCache.ts`)** send no auth header — if `/files`/`/uploads` go behind the bearer, thread `Authorization` into both raw-fetch paths (confirm against the API first; spec hints they stay public).
7. **Launcher→agent token handoff** (largest unbuilt launcher piece): the launcher already holds `session.accessToken` (`launch.ts:249,269`); add `-Dloontail.network.serviceToken=<accessToken>` alongside the existing `-Dloontail.network.serviceUrl` (`launch.ts:121-124`).
8. **Tests:** update `tests/setup/env.ts` and any catalog/bundle test asserting the apiToken bearer.

### 4.2 Agent / Mod (`loontail-minecraft-network-agent`, retired `-mod`)

The agent authenticates with the **opaque `network_sessions` token**, not the Yggdrasil token. At startup it reads `uuid/username/userType` from JVM launch args (deliberately **refusing** `--accessToken` — test-enforced in `LaunchSessionSecurityTest`), calls the unauthenticated `POST /users/bootstrap`, stores the returned token in `NetworkSessionManager`, and attaches it as Bearer on every REST call + the `/signaling` and host `/relay` WS handshakes. Guest `/relay` uses a per-join ticket token (keep distinct). All ~30 network endpoints (friends/presence/world-sessions/join/invites + the two WS) ride this token. The mod is a 1:1 sibling.

**Required changes:**
1. **Become token-aware from the Yggdrasil access token.** Add a `loontail.network.serviceToken` sysprop / `serviceToken` agent-arg / `LOONTAIL_NETWORK_SERVICE_TOKEN` env resolution in `AgentSettings.from()` (the launcher injects it per 4.1#7). Reading `--accessToken` is the riskier fallback that contradicts the test-enforced security invariant — re-scope `SecuritySourceScanTest`/`LaunchSessionSecurityTest` deliberately.
2. **Seed `NetworkSessionManager` with the access token up front** instead of from `BootstrapResponse.token`. Because every Bearer call-site pulls through `session::token`, all of `NetworkApiClient.request()`/`SignalingClient`/`HostRelay` become token-aware with **zero** changes to their Bearer plumbing.
3. **Decide the fate of `POST /users/bootstrap`** — replace with a token-authenticated "who am I"/presence-start call (the backend now identifies the user from the token), or flip it to `auth=true` and stop returning a token. Whatever replaces it **must** still carry `{minecraftVersion, modVersion, loader}` (today's only carrier; the join/invite compatibility gate depends on them) and trigger `presence.start()`.
4. **Handle 401/expired** explicitly — a 15-day token can expire mid-session; today signaling/relay reconnect-loop forever with a stale token. Add a 401-aware re-auth-needed state.
5. **Leave guest-relay ticket auth as-is** (capability token scoped to one join).
6. **Mod**: apply identically only if still shipped; otherwise treat as frozen.

**Tension to rule on:** the design spec declares the NETWORK surface "frozen/unchanged," yet switching it from `network_sessions` to the Yggdrasil token changes that contract's auth model. Needs an explicit decision.

### 4.3 Internal domains (network / catalog / bundles / yggdrasil / textures / admin)

**network** is `api_tokens`-clean (removing `api_tokens` is a no-op here). Migration:
- Replace `AuthUser` → `YggdrasilUser` on **every** authenticated handler (friends/presence/worlds/join_requests/invites/users `me`+`search`; ~27 sites). Both extractors expose `.id()`/`.user`, so bodies are unchanged — only the type + import flip. This **gains** the blocked/confirmed gate `AuthUser` lacks (fixes S2).
- Swap the two WS auth calls: `signaling.rs:34` and `relay.rs:84` (host) from `user_from_token` → `validate_yggdrasil(..., None)`; keep the guest ticket path (`relay.rs:108-149`) as-is.
- **Delete** `crates/network/src/sessions.rs` (sole `network_sessions` writer), `core::user_from_token`, `revoke_all_network_sessions_for_user` (admin password-reset/disable must then rely on `invalidate_all_yggdrasil_for_user`).
- **Blocker:** `FriendRequestRow`/`FriendPresenceRow` have **non-Option** `minecraft_uuid` (`friends.rs:53`, `presence.rs:215`); Yggdrasil-only accounts may lack one until they play online-mode. Either guarantee `profile_uuid`/`minecraft_uuid` assignment at login or relax these to `Option`.

**catalog + bundles:**
- Delete `crates/catalog/src/apitoken.rs` + its re-exports; replace all six `authorize_public_read` calls (`public.rs:30,49,63,83,97,114`) with a `YggdrasilUser` extractor arg (turns today's permissive gate into a real one).
- **Close the open bundle reads:** add `YggdrasilUser` to `get_manifest`/`serve_file` (`bundles/src/public.rs:19,47`) — currently fully unauthenticated (net-new requirement; also see S1). **Behavior change**: any unauthenticated consumer (CDN prefetch, browser direct download, mod self-update) breaks — decide signed-URL/token-query alternative if needed.
- Keep all writes behind `AdminUser` (already correct).
- Delete `crates/admin/src/tokens.rs` + its routes/DTOs/`pub use`; drop the `api_tokens` table via a new migration.
- **CSRF note:** catalog/bundle admin mutations do **not** call `verify_csrf` (only admin users/tokens do) — under the cookie-authorizes-everything model these become CSRF-exposed; fix in lockstep though independent of `api_tokens`.

**yggdrasil + textures** (already the password-login + access-token issuer):
- **Add a public registration endpoint** (e.g. `POST /authserver/register`) creating an `origin='yggdrasil'` user (hash_password + reconcile `profile_uuid`, `is_admin=false`) and returning a session — **no self-signup exists anywhere today** (accounts come only from the password-less mod bootstrap or `admin_create_user`).
- **Add a `YggdrasilAdmin` variant** (`validate_yggdrasil` + `require user.is_admin`) so the admin REST API can be authorized by the same token; keep the SPA httpOnly cookie from the same credentials.
- **Carve out `/join`/`/hasJoined`** from the "token on every request" rule (fixed Mojang wire protocol).
- Consider enforcing `client_token` in `YggdrasilUser` for write/admin paths (today `validate_yggdrasil(..., None)` skips it).
- Add the request-log capture seam where `validate_yggdrasil` already resolves the user (cleanest place to attribute requests).

---

## 5. Observability Plan Inputs

**Capturable now (three disjoint, narrow mechanisms; none capture per-request HTTP logs):**
1. **In-process Prometheus counters** (`crates/core/src/metrics.rs:6-14`) — 7 hand-incremented domain-event `AtomicU64`s (bootstraps/heartbeats/friend_requests/join_tickets/relay_sessions/relay_bytes/signaling_connections), in-memory, reset on restart, **no** method/path/status/latency/user, only network routes covered, exposed at the **unauthenticated** `GET /metrics`. `signaling_connections` is a lifetime total, not a gauge; only the two realtime gauges (`infra.rs:68,72`) are live and they read one node's maps.
2. **`TraceLayer::new_for_http()`** (`main.rs:111`) — ephemeral stdout text only; nothing durable/queryable/in-SPA. The natural seam to augment.
3. **`user_events` table** (`migrations/0008`) — event-typed (not HTTP), read by `analytics::timeseries`; **but its only writer `record_event` is dead in prod** (A1), so it's empty and the dashboard timeseries renders empty. `overview()` computes 5 gauges live off presence/sessions/users (not `user_events`), masking the gap.

**Admin analytics auth:** `GET /admin/analytics/{overview,timeseries}` are `AdminUser`-gated; `/metrics` + `/health` are **public** — a request-log endpoint must sit behind `AdminUser`, never copy the open `/metrics` pattern.

**What the request-log + traffic dashboard needs:**
- **New `request_logs` table** (new migration `0010_request_logs.sql`; do **not** overload `user_events`): `id BIGSERIAL, ts TIMESTAMPTZ, method, path` (route template via `MatchedPath`, not raw query — bound cardinality), `status SMALLINT, latency_ms INT, user_id UUID NULL, auth_kind TEXT (yggdrasil|admin|anon — `network`/`api_token` retire), ip, user_agent, bytes_out`. Indexes: `(ts DESC)` live tail, `(path, ts)`, `(status, ts)`. Plan retention (daily partition or a cleanup task in `spawn_cleanup_tasks`) — request volume ≫ sparse `user_events`.
- **Capture middleware** (`axum::middleware::from_fn` / Tower layer) on the root router around `TraceLayer`, recording method/matched-route/status/wall-clock latency/principal. **Write async off the hot path** (`tokio::spawn` or bounded mpsc + background flusher) — the contract `user_events` already promised but never delivered.
- **Principal attribution:** auth is resolved per-handler, so a global middleware can't see the user — either re-resolve the bearer cheaply or have extractors stash `user_id`+`auth_kind` into request extensions. The single-token redesign collapses this to one resolution path (do it in/after `validate_yggdrasil`).
- **Aggregation endpoints** (AdminUser-gated, runtime sqlx, allowlisted bucket/interval reusing `analytics.rs:74-116`): paginated `GET /admin/analytics/requests` (filters method/path/status/user/since) + bucketed `GET /admin/analytics/requests/timeseries`.
- **Dashboard UI:** new route in `admin-ui/src/App.tsx`, nav entry in `AppShell.tsx` (replacing the retiring "API Tokens"), `useQuery` hook (reuse the 15s `refetchInterval`), Recharts graphs reusing `DashboardPage` scaffolding + a shadcn requests table; camelCase types.
- **Also wire the existing dead writes** (A1): call `record_event` from the real hot paths so the existing bootstraps timeseries stops rendering empty.
- **WS volume:** sample/exclude high-QPS relay/signaling upgrades so the table doesn't explode.
- This is **net-new** work beyond the current spec (which scopes "live in-game data feeding analytics" out of MVP).

---

## 6. Rename Impact: `loontail-minecraft-network-service` → `loontail-launcher-api`

The rename is **already done at the build-artifact level**: bin output is `loontail-launcher-api` (`crates/server/Cargo.toml:10`, `Dockerfile:55/66/78`), compose files declare `name: loontail-launcher-api` with matching `container_name`s. **No** occurrences of `network-service`/`minecraft-network-service`/`network_service` exist in source/Cargo/Docker/CI. What remains:

**Must touch:**
- [ ] **Working-tree directory** `e:/workspace/elixir/loontail-minecraft-network-service` → `loontail-launcher-api` (filesystem/VCS op; not an in-file edit). Also the GitHub repo slug if one exists — this drives the GHCR image path via `${GITHUB_REPOSITORY,,}` (`.github/workflows/deploy.yml:56-58`).
- [ ] `docs/superpowers/specs/2026-06-13-loontail-launcher-api-design.md:29` — the **only** remaining textual `loontail-minecraft-network-service` ("evolve the existing … repo in place").

**Conditional (Postgres DB rebrand `loontail_network` → e.g. `loontail_launcher` — cosmetic; needs a volume reset/manual SQL on a live DB per `deploy-remote.sh:17-25`):**
- [ ] `docker-compose.yml:20,24,44` (POSTGRES_DB / healthcheck / DATABASE_URL)
- [ ] `docker-compose.prod.yml:24,28`
- [ ] `.env.example:16,21`
- [ ] `.github/workflows/deploy.yml:15,102` (comment + `DB_NAME` fallback)
- [ ] `scripts/deploy-remote.sh:11,36`
- [ ] `README.md:38,125`

**Conditional (`NETWORK_DOMAIN` var + brand-stale default `minecraft-network.loontail.dev` → e.g. `launcher-api.loontail.dev`):**
- [ ] `.github/workflows/deploy.yml:13,104,130`
- [ ] `Caddyfile:4,12` (runtime TLS consumer)
- [ ] `docker-compose.prod.yml:8,74,84`
- [ ] `scripts/deploy-remote.sh:13,33,59`

**Optional polish (NOT required — bin output already correct):**
- [ ] `crates/server/Cargo.toml:2` package `loontail-server` → `loontail-launcher-api` (+ `Cargo.lock:1205`, `loontail_server` RUST_LOG target in both compose files, `.env.example:70`, `main.rs:173`). Recommend leaving to avoid churn.

**Explicitly EXCLUDE (do NOT rename):** the `loontail-network` crate / `loontail_network` RUST_LOG target (`docker-compose*.yml`, `.env.example:70`, `main.rs:102,173`, `network_api.rs:29`) — the legitimate network **domain** crate, lexically colliding with the stale DB name but semantically unrelated.

**Open decisions:** (a) is the DB rename in scope or kept for volume stability? (b) keep `NETWORK_DOMAIN` (fix only its value) or rename it across all consumers in lockstep? (c) confirm external `file:` links / launcher/agent base URLs pointing at the old path (outside this repo).

---

## 7. Prioritized Backlog

Severity-tagged, ordered for implementation waves. `[S#]/[A#]` cross-reference §2/§3.

**Wave 0 — Critical security (ship before/with the redesign)**
1. **[HIGH][S1]** Validate `{slug}` + `require_by_slug` on both public bundle endpoints; fix the self-referential `starts_with`; add a traversal test.
2. **[HIGH][S2]** Add `u.blocked = false` to `user_from_token`; revoke network+yggdrasil tokens in the block handler. *(Subsumed by the network→YggdrasilUser swap in Wave 3.)*
3. **[HIGH][S3]** Login throttle (per-account+per-IP backoff/lockout) + global `tower_governor`; `into_make_service_with_connect_info` + trusted-proxy XFF; add `AppError::TooManyRequests`.
4. **[HIGH][S4]** CORS fail-closed default + explicit origin/header/method allowlists; startup assertion against credentials-while-Any.
5. **[HIGH][S6]** Stream multipart uploads chunk-by-chunk with a running cap (bundles archive + single-file + textures).

**Wave 1 — Medium security hardening**
6. **[MED][S5]** Security-headers layer (HSTS/nosniff/frame-deny+CSP/referrer-policy) scoped to `/admin`.
7. **[MED][S13]** `unwrap_or_else(|e| e.into_inner())` (or parking_lot) at the 7 `realtime.rs` lock sites.
8. **[MED][S7+S15]** Constant-time `client_token`/CSRF compare; add a SHA-256 lookup column for the access token (non-Mojang paths).
9. **[MED][S8]** Restrictive perms (0600/0700) on the RSA key file + dir.
10. **[MED][S9/S10/S11]** Admin: `SameSite=Strict` + Origin check on login; revoke tokens on `is_admin`/block transitions; self-protection + last-admin guard.
11. **[MED][S12]** Drop WS `?token=` fallback (or path-only span + redact); lower prod log level.

**Wave 2 — Architecture prerequisites for the redesign**
12. **[HIGH][A1/A6/A10]** Move the event-write seam into `core`; wire real hot paths to emit; design `request_logs` + capture middleware (async off hot path) + AdminUser-gated aggregation endpoints + dashboard UI (§5).
13. **[HIGH][A5]** Introduce the unified `core` extractor (`Authenticated` + `is_admin` variant); neutralize the `Unauthorized` message.
14. **[HIGH][A3]** Reconcile bootstrap: have the live handler call `find_or_create_from_bootstrap` (gives `profile_uuid`).
15. **[HIGH][A4]** Add Biome config + scripts + CI to admin-ui.

**Wave 3 — Single-token unification & api_tokens removal**
16. **[HIGH][A2/A8/S24]** Add public registration; promote `YggdrasilUser` to universal authenticator; add `YggdrasilAdmin`; carve out Mojang `/join`/`/hasJoined`.
17. **[HIGH]** network: swap `AuthUser`→`YggdrasilUser` (~27 sites) + 2 WS calls; delete `sessions.rs`/`user_from_token`/`revoke_all_network_sessions_for_user`; resolve the non-Option `minecraft_uuid` blocker.
18. **[HIGH]** catalog+bundles: delete `apitoken.rs`/`tokens.rs`/routes/DTOs; re-gate the 6 catalog reads + 2 bundle reads on the unified token; add CSRF to catalog/bundle admin mutations; drop the `api_tokens` table (new migration).
19. **[HIGH]** Launcher: remove `API_TOKEN`; add `auth:'yggdrasil'` mode + refresh-and-retry; flip the 2 callers; launcher→agent token handoff (`-Dloontail.network.serviceToken`); update tests/comments.
20. **[HIGH]** Agent: token-aware via injected sysprop; seed `NetworkSessionManager` up front; decide bootstrap replacement (carry version/loader + presence-start); 401-aware re-auth; re-scope the `--accessToken` security tests. **Rule on the "network surface frozen" vs "single token everywhere" tension first.**

**Wave 4 — Low/quality cleanups ("по красоте")**
21. **[MED][A7]** Single-statement `invalidate_all_yggdrasil_for_user`; **[LOW][S20]** transactional refresh.
22. **[LOW][S14/A12]** Escape LIKE wildcards + `ESCAPE '\'`; replace the `'%%'` sentinel; reconcile search page sizes.
23. **[LOW][S16/S17/S18/S19]** RSA blinding; pinned Argon2 params from config; dummy-verify on absent user; 401-vs-403 status corrections.
24. **[LOW][A9]** `Config::validate()` (warn on unparseable, reject nonsensical, explicit CORS-empty + pool size from env).
25. **[LOW][A11/A13/A16/A17/A21]** Remove `///` decorative TS comments; drop dead TTL fallbacks; remove vestigial `yggdrasil_validated_at` + over-`pub` items; log on `let _ =` broadcast errors; align analytics window options.
26. **[INFO][A19]** CI grep guard against `query!`/`query_as!`/`query_scalar!`.

**Wave 5 — Test coverage** *(parallelizable with Wave 3)*
27. **[HIGH]** Integration suites for: network invites (8 handlers), join-requests inWorld approval, relay WS + `try_admit_player` capacity race, catalog admin CRUD (17 handlers); a cross-surface single-token auth regression suite; the new request-log middleware/retention/aggregation.
28. **[MED]** signaling WS presence transitions; bundles admin file-ops/rename/rehash/bulk-delete traversal safety; hasJoined IP + 30s TTL e2e; network-session expiry/revocation parity; end-to-end `build_router` (NormalizePathLayer/CORS/nesting).
29. **[LOW]** `verify_csrf` unit branches; admin-ui LoginPage/RequireAuth/AuthProvider + the new analytics page; textures file-store helpers.

**Wave 6 — Rename** *(low-risk, can land anytime)*
30. **[INFO]** Execute §6: rename the directory/repo + fix the one spec line; decide DB-name and `NETWORK_DOMAIN` rebrand scope and apply in lockstep if chosen; leave the `loontail-network` crate target untouched.
