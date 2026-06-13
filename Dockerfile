# --- Build stage ---------------------------------------------------------
# Rust >= 1.85 is required: a transitive dependency uses edition 2024.
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Cache dependencies: copy the workspace manifests first, build throwaway libs
# so the dependency graph is compiled before the real sources land.
COPY Cargo.toml Cargo.lock* ./
COPY crates/core/Cargo.toml ./crates/core/Cargo.toml
COPY crates/network/Cargo.toml ./crates/network/Cargo.toml
COPY crates/server/Cargo.toml ./crates/server/Cargo.toml
RUN mkdir -p crates/core/src crates/network/src crates/server/src \
    && echo "" > crates/core/src/lib.rs \
    && echo "" > crates/network/src/lib.rs \
    && echo "fn main() {}" > crates/server/src/main.rs \
    && cargo build --release --workspace \
    && rm -rf crates/core/src crates/network/src crates/server/src

# Build the real sources.
COPY migrations ./migrations
COPY crates ./crates
RUN find crates -name '*.rs' -exec touch {} + \
    && cargo build --release --bin loontail-launcher-api

# --- Runtime stage -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/loontail-launcher-api /usr/local/bin/loontail-launcher-api
# Migrations are embedded in the binary, but ship them for reference/tooling.
COPY migrations ./migrations

ENV HTTP_HOST=0.0.0.0
ENV HTTP_PORT=8080
EXPOSE 8080

# Healthcheck hits the same endpoint compose uses.
HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=5 \
    CMD curl -fsS http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/loontail-launcher-api"]
