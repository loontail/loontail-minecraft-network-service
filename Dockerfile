# syntax=docker/dockerfile:1

# --- Admin SPA stage -----------------------------------------------------
# Build the React/Vite admin SPA first. Its output (admin-ui/dist) is embedded
# into the Rust binary at compile time via rust-embed, so the real UI must exist
# before the Rust build runs.
FROM node:22-alpine AS admin-ui

WORKDIR /app/admin-ui

# Install dependencies against the lockfile only first, so the (slow) npm ci
# layer is cached and reused whenever only source files change.
COPY admin-ui/package.json admin-ui/package-lock.json ./
RUN npm ci

COPY admin-ui/ ./
RUN npm run build

# --- Rust build stage ----------------------------------------------------
# rust:1.95 ships edition-2024 support required by a transitive dependency.
FROM rust:1.95-slim-bookworm AS builder

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
    cargo build --release --workspace; \
    rm -rf crates/*/src

# Build the real sources. The admin SPA build output is copied in so rust-embed
# embeds the real UI (not the build.rs placeholder).
COPY migrations ./migrations
COPY crates ./crates
COPY --from=admin-ui /app/admin-ui/dist ./admin-ui/dist
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --bin loontail-launcher-api

# --- Runtime stage -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

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
