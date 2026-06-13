# loontail-launcher-api — MVP design & build contract

Status: **approved, building MVP-1**. Date: 2026-06-13.

This is the single source of truth for every agent working on the build. Read it
fully before touching code. It defines the crate layout, the database schema, the
HTTP/WS contract, shared conventions, the test strategy, and the design system. If
something here conflicts with code you find, this doc wins unless a newer commit
says otherwise.

---

## 0. What we are building

One Rust binary, **`loontail-launcher-api`** (one Postgres), that consolidates what
used to be three systems:

- the existing Rust **network service** (friends / presence / world-sessions /
  join flow / relay / signaling / metrics) — kept as-is,
- the **Yggdrasil** Mojang-compatible auth/session/textures server (ported from the
  Strapi plugin, **online-mode, RSA-SHA1 signed textures**),
- the **skin/cape registry**, the launcher **catalog** (clients/keywords/servers),
  and the **bundle registry** (builds/artifacts/manifests),
- a new **admin panel** (React + shadcn SPA) with user management (create users
  bound to Yggdrasil) and live analytics.

### Locked product decisions (do not relitigate)

1. **Repo**: evolve the existing `loontail-launcher-api` repo in place
   (keep git history + the working Docker/Caddy/CI/Hetzner pipeline). Restructure
   the single crate into a **Cargo workspace**. The bin crate is named
   `loontail-launcher-api`.
2. **Data**: pre-launch, **no migration** — build the schema fresh. No bcrypt
   compatibility constraint ⇒ use **Argon2id** for passwords. No identity
   reconciliation-at-migration; the reconciliation *rule* still applies at runtime.
3. **Minecraft auth**: full **online-mode**. Port the exact RSA-SHA1 (PKCS#1 v1.5),
   4096-bit texture-property signing. **Reuse the existing key verbatim**
   (`data/yggdrasil/keys/active.key.pem` from `loontail-yggdrasil`); generate only
   if absent. Correctness is proven by a **golden-vector test** against Node output.
4. **Admin UI**: React + Vite **SPA served by the backend** under `/admin`, using
   **shadcn (new-york)** with the **launcher's design system** (tokens + `cn.ts`).

### MVP boundary

In scope: the backend itself + admin SPA + Yggdrasil user creation + analytics, all
verifiable by tests / curl / the admin panel. **Out of scope**: wiring the Minecraft
mod, the network agent / authlib-injector launcher flow, repointing the launcher,
live in-game data feeding analytics, S3/MinIO, multi-process relay, rate limiting
beyond a basic guard.

---

## 1. Workspace layout

```
loontail-launcher-api/                  (the existing repo, git history preserved)
├─ Cargo.toml                           # [workspace] members + [workspace.dependencies]
├─ Cargo.lock
├─ migrations/                          # single ordered sqlx migration dir (embedded via sqlx::migrate!)
├─ crates/
│  ├─ core/                             # shared kernel — see §3
│  ├─ yggdrasil-protocol/               # pure protocol lib (Rust analogue of @loontail/yggdrasil-core)
│  ├─ network/                          # ported existing service (friends/presence/relay/signaling/worlds/invites/join)
│  ├─ yggdrasil/                        # /authserver, /sessionserver, /meta + RSA-SHA1 signing
│  ├─ textures/                         # skin/cape registry + PNG validation + static serving
│  ├─ catalog/                          # clients/keywords/servers (+ i18n + media)
│  ├─ bundles/                          # builds/artifacts, ZIP ingest, manifest, on-disk layout
│  ├─ admin/                            # /admin REST + serves the SPA static assets
│  └─ server/                           # bin `loontail-launcher-api`: config → pool → migrate → router → serve
├─ admin-ui/                            # React+Vite+shadcn SPA (own toolchain); build output served by `admin`
├─ data/yggdrasil/keys/                 # RSA keypair (active.key.pem) — copied from loontail-yggdrasil
├─ docs/superpowers/specs/              # this doc
├─ Dockerfile, Caddyfile, docker-compose*.yml, .github/  # reused, updated for the workspace
└─ scripts/
```

### Crate dependency DAG (no cycles)

```
core  ←  network, yggdrasil, textures, catalog, bundles, admin
yggdrasil-protocol  ←  yggdrasil, textures
core, all domains  ←  server (bin)
```

`core` must NOT depend on any domain crate. `yggdrasil-protocol` depends only on
serde/base64/hex/uuid/thiserror (no axum, no sqlx, no rsa) so it unit-tests in
isolation.

### State strategy (pragmatic, no heavy generics)

There is ONE `AppState` and it lives in **`core`**. It holds all shared runtime
state, including the in-memory realtime structures, so domain `routes()` are simply
`Router<AppState>` and domain crates depend only on `core`:

```rust
// core::state
#[derive(Clone)]
pub struct AppState {
    pub pool: sqlx::PgPool,
    pub config: std::sync::Arc<Config>,
    pub metrics: std::sync::Arc<Metrics>,
    pub realtime: std::sync::Arc<Realtime>, // signaling hub + relay rendezvous (in-memory)
}
```

`core::realtime` holds the generic in-memory pieces (per-user mpsc signaling fan-out
map; relay rendezvous `Mutex<HashMap<..>>`) — generic data structures only, no
network business logic, so this stays free of cycles. Each domain crate exposes
`pub fn routes() -> axum::Router<AppState>`; `server` merges them and runs migrations.

---

## 2. Shared conventions (every crate obeys these)

- **sqlx is runtime, not compile-time.** Use `sqlx::query`, `query_as`,
  `query_scalar` — **never `query!`/`query_as!`**. This keeps the workspace
  compiling with no DATABASE_URL and no `.sqlx` cache. The DB is only needed for
  tests.
- **serde wire format is camelCase** for all DTOs:
  `#[serde(rename_all = "camelCase")]`. Preserve field names exactly as the mod/agent
  and launcher contracts specify (see §5). ServerEvent union uses
  `#[serde(tag = "type", rename_all = "camelCase")]`.
- **Dual error shaping.** `core::error::AppError` implements `IntoResponse`. Network
  + admin + catalog + bundle errors serialize as `{"error":{"code","message"}}`
  (message sanitized: strip control chars, cap 200). Yggdrasil errors serialize as
  `{"error","errorMessage","cause?}` with the Mojang status codes (400/401/403/404/
  500, 204 for state-change success). Provide a marker so a handler picks the shape.
- **Three token namespaces, never cross-accepted**: `network_sessions` (opaque,
  SHA-256 of a 256-bit random token, Bearer), `yggdrasil_tokens` (access+client
  64-hex plaintext — Mojang protocol requires verbatim client storage), and
  `admin_sessions` (opaque, SHA-256, httpOnly cookie). Each has its own extractor
  (`AuthUser`, `YggdrasilUser`, `AdminUser`) in `core::auth`.
- **Passwords**: Argon2id via the `argon2` crate, default params. Stored in
  `users.password_hash` (NULL = account cannot password-login until a reset).
- **UUIDs**: `minecraft_uuid` stored dashed canonical; `profile_uuid` stored 32-char
  **undashed lowercase**. Conversion helpers live in `yggdrasil-protocol`.
- **Identity reconciliation rule** (runtime invariant): when a user's
  `minecraft_uuid` M is known, `profile_uuid := undash(M)` (deterministic — one
  account). A random undashed UUID is assigned only when `minecraft_uuid IS NULL`
  (admin-created users get one at creation time). `normalized_username` is UNIQUE.
- **Response caps**: cap response bodies at 4 MiB where buffered; reject empty 2xx
  body on non-void calls per the mod/agent contract.
- **No meaningless comments.** Only `// why` for genuine workarounds/invariants.
  Docs/comments/commit messages in English.
- **Single-process deployment** is an accepted constraint (in-memory relay/signaling
  rendezvous + startup reconciliation). Document it; do not add Redis.

### Workspace dependencies (pin in root `[workspace.dependencies]`)

axum 0.8 (ws, macros, multipart), tokio 1 (full), tower 0.5, tower-http 0.6
(cors, trace, fs), sqlx 0.8 (runtime-tokio, tls-rustls, postgres, uuid, chrono,
macros, migrate), serde 1 (derive), serde_json 1, uuid 1 (v4, serde), chrono 0.4
(serde), thiserror 2, anyhow 1, tracing 0.1, tracing-subscriber 0.3 (env-filter),
sha2 0.10, sha1 0.10, rand 0.8, hex 0.4, dotenvy 0.15, futures-util 0.3,
argon2 0.5, rsa 0.9 (with sha1 for Pkcs1v15Sign), base64 0.22, zip 2,
mime 0.3 / multipart via axum, async-trait as needed. For tests: tower (ServiceExt).

---

## 3. `core` crate (the shared kernel)

Owns everything cross-cutting so domains stay thin and depend only on `core`:

- `state` — `AppState`, `Realtime` (signaling hub: `DashMap<UserId, Vec<mpsc::UnboundedSender<ServerEvent>>>` or `Mutex<HashMap<..>>`; relay rendezvous map). Port the existing network state here.
- `config` — env config (extends the existing one): DB, HTTP, session/heartbeat/world TTLs, **plus** `yggdrasil` (publicUrl, key path, token TTL + per-user cap, skinDomains), `textures` storage root, `bundles` storage root + publicUrl, `admin` (bootstrap admin creds/seed, session TTL, cookie name), `cors`.
- `error` — `AppError` + `IntoResponse` (dual shaping per §2).
- `db` — pool builder + `sqlx::migrate!("../../migrations")` runner + startup reconciliation (close orphan relay_sessions, reset world_sessions.current_players).
- `metrics` — the existing `AtomicU64` counters/gauges + Prometheus text. Add hooks for new counters (yggdrasil auths, texture uploads, bundle ops, admin actions).
- `auth` — the three token tables' issue/verify + extractors `AuthUser`, `YggdrasilUser`, `AdminUser`; Argon2id hash/verify; opaque-token SHA-256 hashing; cookie/CSRF helpers for admin.
- `identity` — the keystone: `find_or_create_*` (mod bootstrap by minecraft_uuid; yggdrasil by username/email+password; admin-created), `assign_profile_uuid`, the reconciliation rule, `normalized_username` handling, user lookup/search/CRUD used by admin.
- `models` — shared row structs + DTOs reused across crates; pagination envelope `{data, meta:{pagination}}` helper; time helpers.

---

## 4. Database schema (single `migrations/` dir, applied at startup + in tests)

Keep the existing network migrations, append new ones. All new tables FK `users(id)`
(UUID). Exact files:

- `0001_init.sql`, `0002_invites.sql` — EXISTING network schema (users, network_sessions, presence, world_sessions, friend_requests, friendships, join_requests, join_tickets, relay_sessions, world_invites). Unchanged.
- `0003_identity.sql` — extend identity:
  ```sql
  ALTER TABLE users
    ADD COLUMN email TEXT,
    ADD COLUMN password_hash TEXT,
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'mod',          -- 'mod' | 'yggdrasil' | 'admin'
    ADD COLUMN profile_uuid TEXT,                            -- 32-char undashed, lowercase
    ADD COLUMN confirmed BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN blocked BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN is_admin BOOLEAN NOT NULL DEFAULT false,
    ADD COLUMN yggdrasil_validated_at TIMESTAMPTZ;
  CREATE UNIQUE INDEX users_email_uniq        ON users (email)        WHERE email IS NOT NULL;
  CREATE UNIQUE INDEX users_profile_uuid_uniq ON users (profile_uuid) WHERE profile_uuid IS NOT NULL;
  CREATE UNIQUE INDEX users_normalized_username_uniq ON users (normalized_username);
  ```
- `0004_yggdrasil_tokens.sql`:
  ```sql
  CREATE TABLE yggdrasil_tokens (
    id           UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    access_token TEXT NOT NULL UNIQUE,         -- 64-hex
    client_token TEXT NOT NULL,                -- 64-hex
    issued_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL
  );
  CREATE INDEX yggdrasil_tokens_user ON yggdrasil_tokens(user_id);
  ```
- `0005_textures.sql` — `skins` (user_id UNIQUE FK CASCADE, profile_uuid, username, file_path, file_url, file_size INT, variant TEXT NOT NULL DEFAULT 'CLASSIC' CHECK in ('CLASSIC','SLIM'), updated_at) and `capes` (same minus variant).
- `0006_catalog.sql` — `catalog_clients`, `catalog_client_locales`, `catalog_media`, `catalog_keywords`, `catalog_keyword_locales`, `catalog_servers`, join tables `catalog_client_keywords`, `catalog_client_servers`. Slugs unique, `published_at` nullable (draft/publish filter), version columns nullable, `bundle_slug` nullable. See §5 CATALOG for the JSON shape these must produce.
- `0007_bundles.sql`:
  ```sql
  CREATE TABLE bundles (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    slug TEXT NOT NULL UNIQUE, name TEXT NOT NULL, description TEXT, version TEXT,
    status TEXT NOT NULL DEFAULT 'draft',       -- draft|processing|ready|failed
    files_count INT NOT NULL DEFAULT 0, total_size BIGINT NOT NULL DEFAULT 0,
    processing_error TEXT, last_generated_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
  );
  CREATE TABLE bundle_artifacts (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    bundle_id UUID NOT NULL REFERENCES bundles(id) ON DELETE CASCADE,
    relative_path TEXT NOT NULL, name TEXT NOT NULL, category TEXT NOT NULL,
    size BIGINT NOT NULL DEFAULT 0, sha256 TEXT, is_dir BOOLEAN NOT NULL DEFAULT false,
    download_once BOOLEAN NOT NULL DEFAULT false, file_modified_at TIMESTAMPTZ
  );
  CREATE INDEX bundle_artifacts_bundle ON bundle_artifacts(bundle_id);
  ```
- `0008_admin_analytics.sql`:
  ```sql
  CREATE TABLE admin_sessions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT NOT NULL UNIQUE, expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
  );
  CREATE TABLE api_tokens (                      -- replaces strapi_api_tokens (launcher catalog/manifest auth)
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL, token_hash TEXT NOT NULL UNIQUE, scopes TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(), last_used_at TIMESTAMPTZ
  );
  CREATE TABLE user_events (                      -- append-only analytics, written async off hot paths
    id BIGSERIAL PRIMARY KEY, user_id UUID REFERENCES users(id),
    event_type TEXT NOT NULL, event_data JSONB, created_at TIMESTAMPTZ NOT NULL DEFAULT now()
  );
  CREATE INDEX user_events_type_time ON user_events(event_type, created_at);
  CREATE INDEX presence_status_hb ON presence(status, last_heartbeat_at);
  ```

`gen_random_uuid()` needs pgcrypto — if not present, add `CREATE EXTENSION IF NOT
EXISTS pgcrypto;` in `0003` (or generate UUIDs app-side). Verify migrations apply
cleanly against a fresh Postgres in the foundation gate.

---

## 5. HTTP / WS contract (frozen where it faces clients)

**NETWORK** (Bearer network token, camelCase) — UNCHANGED from current service; keep
every route, DTO field name, ServerEvent variant, and the WS `/signaling` + `/relay/
{relaySessionId}?role=host|guest` behavior exactly. (Full list in the analysis; the
ported `network` crate keeps them verbatim.)

**YGGDRASIL** (mounted at config `publicUrl`, e.g. `/api/yggdrasil`):
`POST /authserver/{authenticate,refresh,validate,invalidate}`,
`POST /sessionserver/session/minecraft/join`,
`GET  /sessionserver/session/minecraft/hasJoined?username&serverId&ip`,
`GET  /sessionserver/session/minecraft/profile/{uuid}?unsigned`,
`POST /api/profiles/minecraft`, `GET /` (meta: serverName, skinDomains,
`signaturePublickey` = SPKI PEM). Tokens: access+client 64-hex, TTL 15d, cap N/user,
hourly cleanup. Join sessions: in-memory `Map<serverId,{userId,ip,expiresAt}>`, 30s
TTL, single-use atomic take, IP check only when both present, 204 on miss.

**TEXTURES**: `GET /textures/{uuid}` → `{skin?:{url,variant?},cape?:{url}}`;
`GET /textures/{uuid}/{skin|cape}` (PNG bytes); `PUT /textures/{skin|cape}` (multipart
`file` + optional `variant`, Bearer Yggdrasil); `DELETE /textures/{skin|cape}`. Files
stored `data/textures/{skins|capes}/{uuid}-{6byte-hex}.png` (revision busts caches);
served statically.

**CATALOG** (launcher contract 1:1; Bearer `API_TOKEN` or public read):
`GET /api/clients?populate[...]=true&locale=` → `{data:Client[],meta:{pagination}}`,
`GET /api/clients/{id}`, `GET /api/keywords`, `GET /api/servers`. Preserve: Strapi
`populate[field]=true` query parsing, `{data,meta:{pagination}}` envelope,
`publishedAt` draft filter, i18n locale fallback, media `url` left server-relative
(launcher absolutizes), version-field shape. Match field names the launcher coerces.

**BUNDLES**: public `GET /api/bundle-registry/builds/{slug}/manifest`
(`Cache-Control: no-cache`, JSON `Record<category, ManifestEntry[]>`, fields:
`{path,name,size,isDir,sha256?,url?,downloadOnce?}` with sha256/url omitted for dirs,
downloadOnce omitted when false — **byte-shape matters**, launcher hashes the raw
JSON). Static `GET /bundle-registry/builds/{slug}/files/{path}`. Files on disk at
`data/bundle-registry/builds/{slug}/files/{relativePath}` + `artifacts.json`
(atomic .tmp+rename). Admin: `/admin/bundles/*` (list/create/get/update/delete,
upload ZIP, single file, folders, delete/rename/rehash/toggle-downloadOnce,
bulk-delete, validate, regenerate, disk-space). ZIP guards: ≤10GB uncompressed,
≤100k entries, zip-slip/absolute-path rejection, skip `__MACOSX/`, streamed SHA-256,
streamed-to-disk (raise/disable axum body limit per upload route).

**ADMIN** (`AdminUser`, cookie session + CSRF, serves SPA):
`POST /admin/auth/{login,logout}`; `GET /admin/users?q=&page=`;
`POST /admin/users` (create user bound to Yggdrasil: email+password →
`assign profile_uuid`, `confirmed=true`, optional `minecraft_uuid`);
`GET|PATCH|DELETE /admin/users/{id}`;
`POST /admin/users/{id}/{block,unblock,reset-password,revoke-tokens}`;
catalog + bundle admin routes; `GET /admin/api-tokens`, `POST`, `DELETE/{id}`;
`GET /admin/analytics/overview` (playingNow, onlineInNetwork, openWorlds,
activeRelays, totalUsers), `GET /admin/analytics/timeseries?metric=&window=`;
`GET /admin/*` serves the SPA.

**INFRA**: `GET /health` (503 if DB down), `GET /metrics` (Prometheus).

---

## 6. Yggdrasil crypto fidelity (highest risk — gate on a golden vector)

- Reuse the existing PEM key `data/yggdrasil/keys/active.key.pem` (copy from
  `loontail-yggdrasil`); generate a 4096-bit key only if absent. Private key PKCS#8
  PEM; public key SPKI PEM exposed in `/meta.signaturePublickey`.
- Build the textures **value**: JSON `{timestamp(ms), profileId(undashed lc),
  profileName, textures:{SKIN?:{url, metadata?:{model:"slim"}}, CAPE?:{url}}}` with
  **fixed field order via serde structs (never a map)**, then base64-encode. Sign the
  **bytes of the base64 string** (not the raw JSON) with **RSA-SHA1 PKCS#1 v1.5**
  (`rsa::Pkcs1v15Sign::new::<Sha1>()`); signature base64-encoded.
- PNG validation (in `yggdrasil-protocol`): signature `89 50 4E 47 0D 0A 1A 0A`,
  IHDR at offset 8 len 13, big-endian width@20/height@24, skins 64x64 or 64x32, capes
  64x32 only. base64 normalizer: standard alphabet, reject len%4==1, pad to /4.
- **Golden-vector test**: fixed (timestamp, uuid, name, skinUrl) must yield a
  byte-identical `value` and a signature that verifies against the existing SPKI key.
  Capture the Node reference output once (run the Strapi plugin / a tiny node script)
  and assert equality. Yggdrasil is not "done" until this is green.

---

## 7. Admin SPA (`admin-ui/`) — shadcn from the launcher

- Vite + React + TypeScript, Tailwind **v4** (CSS-vars config in `index.css`, no
  config file), shadcn **new-york**, lucide icons — mirror the launcher's
  `components.json`.
- **Reuse the launcher design system**: copy the `@theme` token block from
  `loontail-launcher/src/renderer/index.css` (neutral-dark OKLCH ladder, Nunito,
  radii, type scale, motion) and the `cn.ts` helper
  (`loontail-launcher/src/renderer/shared/lib/cn.ts`, with `extendTailwindMerge`
  registering the custom `text-*` size tokens). Add shadcn components via the CLI
  (`button card table dialog input form badge dropdown-menu sonner chart` etc.).
- Auth: login form → cookie session; all admin API calls same-origin. CSRF token
  flow per `core::auth`. Use TanStack Query for data, Recharts (shadcn `chart`) for
  analytics. Pages: Login, Dashboard (overview gauges + timeseries), Users (table +
  create-Yggdrasil-user dialog + block/reset/revoke), Catalog, Bundles, API tokens.
- Build output (`admin-ui/dist`) is embedded/served by the `admin` crate under
  `/admin` (e.g. `rust-embed` or `tower-http ServeDir`). Keep it a static SPA.

---

## 8. Test strategy (write tests with each crate — non-negotiable)

- **Unit** (no DB): `yggdrasil-protocol` (uuid dash/undash, PNG validation table-
  driven + fuzz the base64 normalizer/PNG header, textures-payload builder), crypto
  golden vector, error shaping, config parsing, identity reconciliation pure logic.
- **Integration** (`#[sqlx::test]`, auto-applies `migrations/`, isolated DB per test):
  every repo/service path. Drive handlers via `tower::ServiceExt::oneshot` against a
  `Router<AppState>` built on the test pool — no real socket needed.
- **Contract tests**: replay the launcher's exact requests (catalog `populate[]` +
  `locale` + Bearer API_TOKEN → assert `{data,meta}` envelope; bundle manifest →
  assert byte-shape + relative URLs; yggdrasil authenticate→join→hasJoined →
  assert signed profile). The yggdrasil online-mode flow gets an end-to-end test.
- **admin-ui**: component/integration tests (Vitest + Testing Library); a smoke test
  that the SPA builds and renders the dashboard against a mocked admin API.
- **Test DB**: Postgres in Docker on **:5433** (host's native PG occupies 5432).
  `DATABASE_URL=postgres://loontail:loontail@localhost:5433/loontail_test`. A
  `scripts/test-db.sh`/compose file spins it up. `#[sqlx::test]` creates per-test DBs.
- CI (`.github/workflows/ci.yml`): `cargo build --workspace`, `cargo test
  --workspace` with a Postgres service container, `cargo fmt --check`, `cargo clippy
  -D warnings`, and an `admin-ui` job (`npm ci && npm run build && npm test`).

---

## 9. Build phases (execution order, each gated)

0. **Foundation** (sequential, must compile + migrate): workspace + `core` +
   `network` (port existing) + `server` + **all domain crates stubbed** (each with
   Cargo.toml, lib.rs, module skeleton, DTO/types, `routes()` returning a real-but-
   empty/`todo!()` router) so the graph is in place and `cargo build --workspace` is
   green; full migration set; copy the yggdrasil key; `admin-ui` scaffold. Gate:
   `cargo build --workspace` green AND migrations apply on docker PG :5433.
1. `yggdrasil-protocol` (pure) + its unit tests.
2. `core::identity` + `core::auth` (Argon2id, three token namespaces, extractors).
3. `yggdrasil` (+ `textures`): online-mode, RSA-SHA1 golden vector, PNG validation.
4. `catalog`: tables + i18n/media + launcher-compatible `/api/clients`.
5. `bundles`: ZIP ingest, manifest byte-shape, on-disk layout.
6. `admin` REST + `analytics`; then the `admin-ui` SPA build-out.
7. Integration + contract tests across the workspace; CI green.

Because the foundation pre-creates every crate dir, Cargo.toml, route wiring, and the
full schema, phases 1–6 edit **disjoint files** and can be parallelized with
git-worktree isolation, each agent owning one crate, merged back behind a `cargo
build`/`cargo test` gate.
