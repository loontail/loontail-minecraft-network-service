#!/usr/bin/env bash
# Roll out loontail-minecraft-network-service on the Hetzner host.
#
# Streamed to the server over SSH (`bash -s`) by .github/workflows/deploy.yml.
# Inputs arrive via the environment:
#   IMAGE          pinned GHCR image ref (ghcr.io/<owner>/<repo>:sha-<commit>)
#   REGISTRY_USER  GHCR username (the workflow actor)
#   REGISTRY_TOKEN ephemeral GHCR token (valid only for the deploy run)
#   DEPLOY_DIR     directory holding docker-compose.prod.yml and .env.prod
# Secrets/tunables are read from $DEPLOY_DIR/.env.prod (never committed).
set -euo pipefail

: "${IMAGE:?IMAGE is required}"
: "${REGISTRY_USER:?REGISTRY_USER is required}"
: "${REGISTRY_TOKEN:?REGISTRY_TOKEN is required}"
: "${DEPLOY_DIR:?DEPLOY_DIR is required}"

cd "$DEPLOY_DIR"

if [ ! -f .env.prod ]; then
  echo "Missing $DEPLOY_DIR/.env.prod (one-time server setup — see deploy.yml header)." >&2
  exit 1
fi

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
