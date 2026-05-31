# --- Build stage ---------------------------------------------------------
# Rust >= 1.85 is required: a transitive dependency uses edition 2024.
FROM rust:1-slim-bookworm AS builder

WORKDIR /app

# Cache dependencies: copy manifests first, build a throwaway lib.
COPY Cargo.toml Cargo.lock* ./
RUN mkdir src \
    && echo "fn main() {}" > src/main.rs \
    && cargo build --release \
    && rm -rf src

# Build the real sources.
COPY migrations ./migrations
COPY src ./src
RUN touch src/main.rs && cargo build --release

# --- Runtime stage -------------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/loontail-minecraft-network-service /usr/local/bin/loontail-minecraft-network-service
# Migrations are embedded in the binary, but ship them for reference/tooling.
COPY migrations ./migrations

ENV HTTP_HOST=0.0.0.0
ENV HTTP_PORT=8080
EXPOSE 8080

# Healthcheck hits the same endpoint compose uses.
HEALTHCHECK --interval=15s --timeout=3s --start-period=20s --retries=5 \
    CMD curl -fsS http://localhost:8080/health || exit 1

ENTRYPOINT ["/usr/local/bin/loontail-minecraft-network-service"]
