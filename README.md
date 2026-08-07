# loontail-launcher-api

Single Rust backend (one binary, `loontail-launcher-api`, one Postgres) for the
Loontail launcher and Minecraft network. It consolidates the network service
(user bootstrap, friends, presence/statuses, world sessions, join
requests/tickets, invites, signaling and relay/tunnel), a Mojang-compatible
Yggdrasil auth/session/textures server, the skin/cape and launcher catalog/bundle
registries, and a React admin SPA (embedded and served under `/admin`). Postgres
for storage. Docker-first for local dev and Hetzner production.

It is **not** a Minecraft server. It stores network/social state and helps players
connect to each other's local worlds through a relay tunnel. It never receives,
stores or logs a Minecraft access token.

The Cargo workspace lives under `crates/` (`core`, `yggdrasil-protocol`, `network`,
`yggdrasil`, `textures`, `catalog`, `bundles`, `admin`, `server`). The admin SPA
lives in `admin-ui/` and is embedded into the binary at compile time via
`rust-embed`, so the Docker build runs `npm run build` before the Rust build.

## Design invariants

Decisions that are cheap to break and expensive to rediscover:

- **Argon2id** for password hashing; session and Yggdrasil tokens are stored as
  SHA-256 digests, never in the clear.
- **Online-mode signatures use RSA-SHA1** — that is what the vanilla client
  verifies for the `textures` profile property. Not a modernisable choice.
- **Runtime sqlx** (`sqlx::query`/`query_as`), not the compile-time `query!`
  macros: the build must not require a live database.
- **One opaque session token** for everything (migration `0010_unify_sessions.sql`):
  the launcher/mod Bearer and the admin cookie are the same `sessions` row, and
  `POST /users/bootstrap` no longer mints anything.

## Run locally (Docker Compose)

Everything in one stack — Postgres, migrations, API and the embedded admin SPA:

```bash
cp .env.example .env
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

- HTTP API: <http://localhost:8080>
- Health: <http://localhost:8080/health>
- Metrics: <http://localhost:8080/metrics>
- Admin SPA: <http://localhost:8080/admin>

Migrations run automatically at startup (embedded via `sqlx::migrate!`), so the
schema is applied on first boot against the Postgres container — no manual step.

The container's database URL is built from `POSTGRES_USER`/`POSTGRES_PASSWORD`/
`POSTGRES_DB` and always targets the `postgres` service, so a host-oriented
`DATABASE_URL` in `.env` (needed for the host-side mode below) cannot break it.

### Run the service on the host (faster iteration, no image rebuild)

The API runs from `cargo`, Postgres stays in a container:

```bash
docker compose -f docker-compose.yml -f docker-compose.dev.yml up -d postgres
DATABASE_URL=postgres://loontail:loontail@localhost:5432/loontail_launcher cargo run
```

`docker-compose.dev.yml` publishes Postgres on 5432 for exactly this. Put that
same URL in `.env` to skip the inline variable. Note the admin SPA is embedded at
compile time, so after changing `admin-ui/` run `npm run build` in it, then touch
`crates/admin/src/spa.rs` before rebuilding.

## Verify

The four gates CI enforces (`.github/workflows/ci.yml`) — a PR that fails any of
them is rejected:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
DATABASE_URL=postgres://loontail:loontail@localhost:5432/loontail_test cargo test --workspace
cd admin-ui && npm run build && npm test
```

`cargo test` needs a reachable Postgres: the tests use `#[sqlx::test]`, which
creates and drops a database per test from the `DATABASE_URL` connection. Point it
at a **scratch** database (`loontail_test` above), not your dev one. The
`docker-compose.dev.yml` Postgres container works for this.

## Deployment (Hetzner)

Rollout is automated; there is no manual build on the server.

**One-time server setup**

1. Docker Engine + the compose plugin + `curl`.
2. A DNS **A record** for `NETWORK_DOMAIN` pointing at the host. If the domain
   sits behind Cloudflare it must be **DNS-only / grey-cloud**, or Caddy's
   ACME HTTP-01 challenge fails and the relay WebSocket gets proxied
   (see [`Caddyfile`](Caddyfile)).
3. Ports 80/tcp and 443/tcp+udp open in the firewall (Caddy: ACME + HTTPS).

**Repository secrets** (`.github/workflows/deploy.yml`)

| Secret | Required | Default |
|---|---|---|
| `HETZNER_HOST` | yes | — |
| `HETZNER_USER` | yes | — |
| `HETZNER_SSH_KEY` | yes | PEM, no passphrase |
| `DB_PASSWORD` | yes | — |
| `HETZNER_SSH_PORT` | no | `22` |
| `HETZNER_DEPLOY_DIR` | no | `/opt/loontail/loontail-launcher-api` |
| `DB_USER` | no | `loontail` |
| `DB_NAME` | no | `loontail_launcher` |
| `NETWORK_DOMAIN` | no | `launcher-api.loontail.dev` |

**Rollout**

Push to `main` → CI goes green → `deploy.yml` builds and pushes
`ghcr.io/<owner>/<repo>:sha-<12>`, ships `docker-compose.prod.yml` + `Caddyfile`
to the deploy dir, writes `.env.prod` there, and runs
`docker compose -f docker-compose.prod.yml --env-file .env.prod up -d`, then
probes `/health` for 60s. `.env.prod` is fully managed by
[`scripts/deploy-remote.sh`](scripts/deploy-remote.sh) — do not hand-author it.

Manual escape hatch on the server (the compose file lives only in the deploy dir,
and `-f` is not optional — `docker-compose.yml` is the local *build* stack and has
no TLS):

```bash
cd /opt/loontail/loontail-launcher-api
docker compose -f docker-compose.prod.yml --env-file .env.prod up -d
```

> **First-init only:** Postgres honours `POSTGRES_USER`/`POSTGRES_DB`/
> `POSTGRES_PASSWORD` only when it initialises an empty volume. Changing
> `DB_USER`/`DB_NAME`/`DB_PASSWORD` after the first successful deploy does **not**
> update the database — see the WARNING block in `scripts/deploy-remote.sh`.

Postgres data lives in the `postgres_data` named volume and app state (Yggdrasil
keys, textures, bundle files) in `app_data`; both survive restarts.
`restart: unless-stopped` keeps the stack up across reboots.

## Required environment

See [`.env.example`](.env.example), which documents every variable the code reads
with its real default. Only `DATABASE_URL` is mandatory. Key knobs:

| Variable | Meaning |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `HTTP_PORT` | HTTP API + relay/signaling WebSocket port (default 8080) |
| `TRUSTED_PROXY` | Take the client IP from `X-Forwarded-For`/`X-Real-IP` (default `false`). **Required behind Caddy/nginx/Cloudflare**, or per-IP rate limiting collapses to one global bucket |
| `METRICS_TOKEN` | Bearer that authorizes `GET /metrics` for a scraper (unset ⇒ admin session only) |
| `SESSION_TTL_SECONDS` | Unified session token lifetime (issued by the Yggdrasil account login) |
| `HEARTBEAT_TIMEOUT_SECONDS` | Older heartbeat → user considered offline |
| `MAX_PLAYERS_PER_WORLD` | Per-world capacity (default 5) |
| `JOIN_REQUEST_TTL_SECONDS` / `JOIN_TICKET_TTL_SECONDS` / `INVITE_TTL_SECONDS` | Join/invite flow TTLs |
| `RATE_LIMIT_MAX_ATTEMPTS` / `RATE_LIMIT_WINDOW_SECONDS` | Credential-endpoint limiter (defaults 10 / 60) |
| `YGGDRASIL_PUBLIC_URL` / `YGGDRASIL_KEY_PATH` / `YGGDRASIL_SKIN_DOMAINS` | Yggdrasil mount path, RSA signing key, advertised skin domains |
| `TEXTURES_STORAGE_ROOT` / `CATALOG_MEDIA_STORAGE_ROOT` | Skin/cape and catalog-media storage roots |
| `BUNDLES_STORAGE_ROOT` / `BUNDLES_PUBLIC_URL` | Bundle-registry storage root + public URL |
| `ADMIN_BOOTSTRAP_USERNAME` / `ADMIN_BOOTSTRAP_PASSWORD` | Seed admin (created on startup only when a password is set) |
| `NETWORK_DOMAIN` | Production only: the domain Caddy serves. Hard-fails `docker-compose.prod.yml` when unset |

Both compose files pass an explicit allow-list of variables into the container, so
a variable set in `.env`/`.env.prod` that is not in their `environment:` block has
no effect under Docker (`.env.example` lists the current gaps).

## Ports & firewall

- **80/tcp** and **443/tcp+udp** — Caddy (ACME challenge + HTTPS). The production
  requirement; only these need to be public.
- **8080/tcp** — the service itself: HTTP API plus the relay and signaling
  WebSockets (`/relay/:id`, `/signaling`), which share this port. Published by
  compose for direct/non-TLS access; debug-only in production, since Caddy reaches
  it over the compose network.

### DNS notes

- `NETWORK_DOMAIN` must be a **DNS-only / grey-cloud** A record if it is behind
  Cloudflare: the orange-cloud proxy breaks Caddy's ACME HTTP-01 challenge and
  proxies the Minecraft relay WebSocket.

## API overview

Authentication:

- The network/social endpoints below require
  `Authorization: Bearer <sessionToken>` — the single opaque session token minted
  by the Yggdrasil account login (`POST /api/auth/login`) and injected into the
  game by the launcher. `POST /users/bootstrap` is included: it is authenticated
  and issues no token of its own. The catalog and bundle-registry routes take the
  same Bearer.
- `GET /health` is unauthenticated. `GET /metrics` needs an authenticated admin
  session or the configured `METRICS_TOKEN` as a Bearer.
- Intentionally anonymous: the Mojang-protocol Yggdrasil endpoints under the
  Yggdrasil mount (authlib-injector and the game call them without a session),
  `POST /api/auth/login` + `POST /api/auth/register`, and the texture read routes
  (`GET /textures/:uuid`, `GET /textures/:uuid/:kind`) the game client fetches
  skins from.
- `/admin/**` (REST + SPA) is gated by the admin session cookie plus a CSRF
  double-submit, not by a Bearer.

```
GET    /health
GET    /metrics
POST   /api/auth/register                  # anonymous: create account, issue session token
POST   /api/auth/login                     # anonymous: issue session token
POST   /api/auth/refresh                   # rotate the session token
POST   /api/auth/logout
GET    /api/auth/me
POST   /users/bootstrap            # bind Minecraft identity + version/loader to the session
GET    /me
GET    /users/search?q=
GET    /friends
POST   /friends/requests           # { toUserId }
GET    /friends/requests/incoming
GET    /friends/requests/outgoing
POST   /friends/requests/:id/accept
POST   /friends/requests/:id/decline
DELETE /friends/:userId
POST   /presence/heartbeat
POST   /presence/status            # { status: online|inWorld|joinable, currentWorldSessionId? }
GET    /presence/friends
POST   /world-sessions             # { maxPlayers? }
PATCH  /world-sessions/:id         # { maxPlayers?, status? }
DELETE /world-sessions/:id
POST   /world-sessions/:id/join-ticket    # joinable: direct ticket
POST   /world-sessions/:id/join-requests  # inWorld: request approval
GET    /join-requests/incoming
POST   /join-requests/:id/accept
POST   /join-requests/:id/decline
POST   /world-sessions/:id/invites        # host (or trusted friend) invites a player
GET    /invites/incoming
GET    /invites/outgoing
GET    /invites/pending-approval          # host queue for friend-of-friend invites
POST   /invites/:id/accept                # returns a join ticket
POST   /invites/:id/decline
POST   /invites/:id/approve
DELETE /invites/:id
GET    /signaling?token=                   # WebSocket: server→client events
GET    /relay/:relaySessionId?token=&role=host|guest   # WebSocket: byte tunnel
```

The two WebSocket routes prefer the `Authorization: Bearer` header and accept
`?token=` only as a fallback (browsers cannot set headers on a WebSocket).

## Migrations

Schema lives in [`migrations/`](migrations). It is embedded in the binary and
applied on startup. To apply manually with the sqlx CLI:

```bash
cargo install sqlx-cli --no-default-features --features postgres
DATABASE_URL=postgres://loontail:loontail@localhost:5432/loontail_launcher sqlx migrate run
```

## Notes / future work

- **Identity (out of scope for MVP):** identity is taken from the Minecraft
  session and sent by the mod. A modified client could spoof `minecraftUuid` /
  `username`. If needed later, add a verified session proof; today this is out
  of scope and not implemented.
- **Abuse protection:** a per-IP sliding-window limit guards the unauthenticated
  credential endpoints (`/admin/auth/login`, `/api/auth/login`,
  `/api/auth/register`, Yggdrasil `authserver/authenticate` + `refresh`) —
  `RATE_LIMIT_MAX_ATTEMPTS`/`RATE_LIMIT_WINDOW_SECONDS`, defaults 10/60, keyed on
  the transport peer unless `TRUSTED_PROXY=true`. State is in-process and lost on
  restart. Per-user quotas and relay bandwidth controls are unimplemented.
- **Relay:** the relay is a WebSocket rendezvous (one pair per guest), in
  `crates/network/src/relay.rs`. A raw-TCP relay listener is unimplemented.
