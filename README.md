# loontail-launcher-api

Single Rust backend (one binary, `loontail-launcher-api`, one Postgres) for the
Loontail launcher and Minecraft network. It consolidates the network service
(user bootstrap, network session tokens, friends, presence/statuses, world
sessions, join requests/tickets, signaling and relay/tunnel), a Mojang-compatible
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

## Run locally (Docker Compose)

```bash
cp .env.example .env
docker compose -f docker-compose.yml -f docker-compose.dev.yml up --build
```

- HTTP API: <http://localhost:8080>
- Health: <http://localhost:8080/health>
- Metrics: <http://localhost:8080/metrics>

Migrations run automatically at startup (embedded via `sqlx::migrate!`), so the
schema is applied on first boot against the Postgres container — no manual step.

### Run the service on the host (against the compose Postgres)

```bash
# Postgres is published on 5432 by docker-compose.dev.yml
DATABASE_URL=postgres://loontail:loontail@localhost:5432/loontail_network cargo run
```

## Run on Hetzner

1. Install Docker + Docker Compose plugin on the server.
2. Copy the repo, create `.env.prod` from `.env.example` (strong `POSTGRES_PASSWORD`,
   correct `DATABASE_URL`, real `CORS_ALLOWED_ORIGINS`).
3. Start it:

   ```bash
   docker compose --env-file .env.prod up -d --build
   ```

Postgres data lives in the `postgres_data` named volume and survives restarts.
`restart: unless-stopped` keeps the stack up across reboots.

## Required environment

See [`.env.example`](.env.example). Only `DATABASE_URL` is mandatory; everything
else has a default. Key knobs:

| Variable | Meaning |
|---|---|
| `DATABASE_URL` | Postgres connection string |
| `HTTP_PORT` | HTTP API + relay/signaling WebSocket port (default 8080) |
| `SESSION_TTL_SECONDS` | Network session token lifetime |
| `HEARTBEAT_TIMEOUT_SECONDS` | Older heartbeat → user considered offline |
| `MAX_PLAYERS_PER_WORLD` | Per-world capacity (default 5) |
| `JOIN_REQUEST_TTL_SECONDS` / `JOIN_TICKET_TTL_SECONDS` | Join flow TTLs |
| `YGGDRASIL_PUBLIC_URL` / `YGGDRASIL_KEY_PATH` / `YGGDRASIL_SKIN_DOMAINS` | Yggdrasil mount path, RSA signing key, advertised skin domains |
| `TEXTURES_STORAGE_ROOT` | Skin/cape storage root (default `data/textures`) |
| `BUNDLES_STORAGE_ROOT` / `BUNDLES_PUBLIC_URL` | Bundle-registry storage root + public URL |
| `ADMIN_BOOTSTRAP_USERNAME` / `ADMIN_BOOTSTRAP_PASSWORD` | Seed admin (created on startup only when a password is set) |

## Ports & firewall

- **8080/tcp** — HTTP API. In this MVP the relay and signaling WebSockets share
  this port (`/relay/:id`, `/signaling`).
- Open 8080/tcp (or your reverse-proxy port) in the Hetzner firewall.

### DNS notes

- The HTTP API may sit behind Cloudflare (orange-cloud proxy is fine).
- Relay/tunnel traffic should use a **DNS-only** record / direct TCP. Do **not**
  proxy Minecraft relay traffic through Cloudflare's HTTP proxy.

## API overview

All endpoints except `POST /users/bootstrap`, `GET /health` and `GET /metrics`
require `Authorization: Bearer <networkSessionToken>`.

```
GET    /health
GET    /metrics
POST   /users/bootstrap            # create/update user from Minecraft session, issue token
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
GET    /signaling?token=                   # WebSocket: server→client events
GET    /relay/:relaySessionId?token=&role=host|guest   # WebSocket: byte tunnel
```

## Migrations

Schema lives in [`migrations/`](migrations). It is embedded in the binary and
applied on startup. To apply manually with the sqlx CLI:

```bash
cargo install sqlx-cli --no-default-features --features postgres
DATABASE_URL=postgres://loontail:loontail@localhost:5432/loontail_network sqlx migrate run
```

## Notes / future work

- **Identity (out of scope for MVP):** identity is taken from the Minecraft
  session and sent by the mod. A modified client could spoof `minecraftUuid` /
  `username`. If needed later, add a verified session proof; today this is out
  of scope and not implemented.
- **Abuse protection (out of scope for MVP):** no rate/quotas/bandwidth limits
  yet. The module layout (per-user state, relay sessions) leaves room to add
  rate limits, per-user/per-IP limits and relay load controls later.
- **Relay:** the relay is a WebSocket rendezvous (one pair per guest). A raw-TCP
  relay listener can be added later; see `src/relay/mod.rs` TODO.
