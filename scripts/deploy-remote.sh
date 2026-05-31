#!/usr/bin/env bash
# Roll out loontail-minecraft-network-service on the Hetzner host.
#
# Streamed to the server over SSH (`bash -s`) by .github/workflows/deploy.yml.
# Inputs arrive via the environment:
#   IMAGE          pinned GHCR image ref (ghcr.io/<owner>/<repo>:sha-<commit>)
#   REGISTRY_USER  GHCR username (the workflow actor)
#   REGISTRY_TOKEN ephemeral GHCR token (valid only for the deploy run)
#   DEPLOY_DIR     directory holding docker-compose.prod.yml and .env.prod
#   DB_USER        Postgres username (default: loontail)
#   DB_NAME        Postgres database name (default: loontail_network)
#   DB_PASSWORD    Postgres password (required)
# All values are written into .env.prod on each deploy so the server stays
# in sync and manual `docker compose up` works without extra env vars.
set -euo pipefail

: "${IMAGE:?IMAGE is required}"
: "${REGISTRY_USER:?REGISTRY_USER is required}"
: "${REGISTRY_TOKEN:?REGISTRY_TOKEN is required}"
: "${DEPLOY_DIR:?DEPLOY_DIR is required}"
: "${DB_PASSWORD:?DB_PASSWORD is required}"

DB_USER="${DB_USER:-loontail}"
DB_NAME="${DB_NAME:-loontail_network}"

cd "$DEPLOY_DIR"

# Write/update all deploy-managed keys in .env.prod.
upsert() {
  local key="$1" value="$2"
  if grep -q "^${key}=" .env.prod 2>/dev/null; then
    sed -i "s|^${key}=.*|${key}=${value}|" .env.prod
  else
    echo "${key}=${value}" >> .env.prod
  fi
}

# Create .env.prod if it doesn't exist yet (fully managed by CI).
touch .env.prod
chmod 600 .env.prod

upsert LOONTAIL_IMAGE  "${IMAGE}"
upsert POSTGRES_USER     "${DB_USER}"
upsert POSTGRES_DB       "${DB_NAME}"
upsert POSTGRES_PASSWORD "${DB_PASSWORD}"
upsert DATABASE_URL      "postgres://${DB_USER}:${DB_PASSWORD}@postgres:5432/${DB_NAME}"

compose() {
  docker compose -f docker-compose.prod.yml --env-file .env.prod "$@"
}

echo "$REGISTRY_TOKEN" | docker login ghcr.io -u "$REGISTRY_USER" --password-stdin
trap 'docker logout ghcr.io >/dev/null 2>&1 || true' EXIT

export LOONTAIL_IMAGE="$IMAGE"

echo "Pulling $IMAGE ..."
compose pull

echo "Starting containers ..."
compose up -d --remove-orphans

echo "Waiting for /health ..."
for _ in $(seq 1 30); do
  if curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
    echo "Service healthy: $IMAGE"
    docker image prune -f >/dev/null 2>&1 || true
    exit 0
  fi
  sleep 2
done

echo "Service did not become healthy within 60s — rolling deploy failed." >&2
compose logs --tail=120 service >&2 || true
exit 1
