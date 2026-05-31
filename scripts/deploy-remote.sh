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
#
# WARNING — Postgres honours POSTGRES_USER / POSTGRES_DB / POSTGRES_PASSWORD
# ONLY when it first initialises an empty data volume. Changing DB_USER /
# DB_NAME / DB_PASSWORD after the first successful deploy will NOT update the
# existing database; the service then fails with "database ... does not exist"
# or "password authentication failed". To change them you must either reset the
# volume (destroys data):
#     docker compose -f docker-compose.prod.yml --env-file .env.prod down -v
#     docker compose -f docker-compose.prod.yml --env-file .env.prod up -d
# or apply the change inside Postgres by hand (ALTER ROLE / CREATE DATABASE).
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
logs="$(compose logs --tail=120 service 2>&1 || true)"
printf '%s\n' "$logs" >&2

# The most common cause is a stale Postgres volume initialised with different
# credentials than the current DB_* secrets (Postgres ignores them after first
# init). Surface a precise next step instead of a bare timeout.
if printf '%s' "$logs" | grep -qiE 'does not exist|password authentication failed|role .* does not exist'; then
  echo "" >&2
  echo "HINT: Postgres credentials/database do not match the existing data volume." >&2
  echo "      POSTGRES_USER/DB/PASSWORD only apply on first volume init. If this is" >&2
  echo "      a fresh deploy with no data, reset the volume and redeploy:" >&2
  echo "        cd $DEPLOY_DIR" >&2
  echo "        docker compose -f docker-compose.prod.yml --env-file .env.prod down -v" >&2
  echo "        docker compose -f docker-compose.prod.yml --env-file .env.prod up -d" >&2
fi
exit 1
