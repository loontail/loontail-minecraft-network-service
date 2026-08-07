# syntax=docker/dockerfile:1

# --- Admin SPA stage -----------------------------------------------------
# Build the React/Vite admin SPA first. Its output (admin-ui/dist) is embedded
# into the Rust binary at compile time via rust-embed, so the real UI must exist
# before the Rust build runs.
# Digest-pinned so "rebuild commit X" reproduces the image. Refresh with
# `docker buildx imagetools inspect node:24-alpine`; Dependabot's docker
# ecosystem bumps it under review.
FROM node:24-alpine@sha256:d32cdf619f63fe0471182d08996dd516c6275bb5fd31ae06e55a570bd9e1ad43 AS admin-ui

WORKDIR /app/admin-ui

# Install dependencies against the lockfile only first, so the (slow) npm ci
# layer is cached and reused whenever only source files change.
COPY admin-ui/package.json admin-ui/package-lock.json ./
RUN npm ci

COPY admin-ui/ ./
RUN npm run build

# --- Rust build stage ----------------------------------------------------
# Pinned to the multi-arch OCI index digest so the builder toolchain cannot drift
# under the floating tag. `rust-version = "1.94"` (sqlx 0.9) is the real floor;
# refresh the digest with `docker buildx imagetools inspect rust:1.95-slim-bookworm`.
FROM rust:1.95-slim-bookworm@sha256:d7482085ff5b415f84dba5647ae71606650bdef00db7aeb69f4b3d170c3e4082 AS builder

WORKDIR /app

# Cache the dependency graph: copy the workspace manifests + the lockfile and
# build throwaway crates so all third-party deps compile in a layer that only
# changes when a Cargo.toml/Cargo.lock changes.
COPY Cargo.toml Cargo.lock ./
COPY crates/core/Cargo.toml ./crates/core/Cargo.toml
COPY crates/yggdrasil-protocol/Cargo.toml ./crates/yggdrasil-protocol/Cargo.toml
COPY crates/network/Cargo.toml ./crates/network/Cargo.toml
COPY crates/yggdrasil/Cargo.toml ./crates/yggdrasil/Cargo.toml
COPY crates/textures/Cargo.toml ./crates/textures/Cargo.toml
COPY crates/catalog/Cargo.toml ./crates/catalog/Cargo.toml
COPY crates/bundles/Cargo.toml ./crates/bundles/Cargo.toml
COPY crates/admin/Cargo.toml ./crates/admin/Cargo.toml
COPY crates/admin/build.rs ./crates/admin/build.rs
COPY crates/server/Cargo.toml ./crates/server/Cargo.toml
RUN set -eux; \
    for c in core yggdrasil-protocol network yggdrasil textures catalog bundles admin; do \
        mkdir -p "crates/$c/src"; \
        echo "" > "crates/$c/src/lib.rs"; \
    done; \
    mkdir -p crates/server/src; \
    echo "fn main() {}" > crates/server/src/main.rs; \
    cargo build --locked --release --workspace; \
    rm -rf crates/*/src

# Build the real sources. The admin SPA build output is copied in so rust-embed
# embeds the real UI (not the build.rs placeholder).
COPY migrations ./migrations
COPY crates ./crates
COPY --from=admin-ui /app/admin-ui/dist ./admin-ui/dist
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --locked --release --bin loontail-launcher-api

# --- Runtime stage -------------------------------------------------------
# Digest-pinned like the builder: the runtime CA trust store must not change
# under a rebuild of the same commit. `apt-get` below still picks up security
# updates for the packages it installs.
FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241 AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/loontail-launcher-api /usr/local/bin/loontail-launcher-api
# Migrations are embedded in the binary and applied at startup; ship them too
# for reference/tooling.
COPY migrations ./migrations

ENV HTTP_HOST=0.0.0.0
ENV HTTP_PORT=8080
EXPOSE 8080

HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=5 \
    CMD curl -fsS http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/loontail-launcher-api"]
