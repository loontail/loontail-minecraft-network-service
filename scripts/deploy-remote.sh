#!/usr/bin/env bash
# Roll out loontail-launcher-api on the Hetzner host.
#
# Streamed to the server over SSH (`bash -s`) by .github/workflows/deploy.yml.
# Every input arrives as an `export NAME=<printf %q>` prologue prepended to this
# script on stdin — never as an ssh command prefix, which the remote shell
# re-parses:
#   IMAGE          DIGEST-pinned GHCR image ref (ghcr.io/<owner>/<repo>@sha256:...)
#   REGISTRY_USER  GHCR username (the workflow actor)
#   REGISTRY_TOKEN ephemeral GHCR pull token (valid only for the deploy run)
#   DEPLOY_DIR     directory holding docker-compose.prod.yml and .env.prod
#   DB_USER        Postgres username (default: loontail)
#   DB_NAME        Postgres database name (default: loontail_launcher)
#   DB_PASSWORD    Postgres password (required)
#   NETWORK_DOMAIN public domain Caddy serves over HTTPS (required)
# The deploy-managed keys are rewritten in .env.prod on each deploy so the server
# stays in sync and manual `docker compose up` works without extra env vars. Keys
# this script does not manage (METRICS_TOKEN, ADMIN_BOOTSTRAP_PASSWORD, RUST_LOG,
# ...) are preserved verbatim.
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
: "${NETWORK_DOMAIN:?NETWORK_DOMAIN is required}"

DB_USER="${DB_USER:-loontail}"
DB_NAME="${DB_NAME:-loontail_launcher}"

# why: GHCR tags are mutable, so `:sha-<commit>` only looks immutable — anything
# with packages:write can repoint it and this host would pull it on the next
# restart. A digest ref is content-addressed: `docker pull` verifies it, so the
# deploy log is evidence of what actually ran. Refuse anything else.
case "$IMAGE" in
  *@sha256:*) ;;
  *)
    echo "IMAGE must be digest-pinned (ghcr.io/<owner>/<repo>@sha256:...), got: $IMAGE" >&2
    exit 1
    ;;
esac

# DATABASE_URL is assembled by string concatenation below, so a password holding
# a URI-reserved character would produce a URL Postgres parses differently (or
# not at all). Newlines in any managed value would inject a second line into
# .env.prod. Both are silent-corruption classes — reject them at the door.
for _var in IMAGE DB_USER DB_NAME DB_PASSWORD NETWORK_DOMAIN; do
  case "${!_var}" in
    *$'\n'* | *$'\r'*)
      echo "$_var contains a newline — refusing to write .env.prod." >&2
      exit 1
      ;;
  esac
done
# Rejects the gen-delims and the characters RFC 3986 does not permit in a URI
# userinfo field at all. Sub-delims (! $ & ' ( ) * + , ; =) stay allowed: they are
# legal there and nothing here interpolates a value into code any more.
case "$DB_PASSWORD" in
  *[/:@?\#\[\]%\ \|\\\"\<\>^\{\}\`]*)
    echo "DB_PASSWORD contains a character a URI userinfo field cannot carry unencoded:" >&2
    echo "  / : @ ? # [ ] % | \\ \" < > ^ { } \` or a space" >&2
    echo "DATABASE_URL is assembled by concatenation, so such a password would produce a URL" >&2
    echo "Postgres parses differently or rejects. Rotate the password." >&2
    exit 1
    ;;
esac

cd "$DEPLOY_DIR"

# Keys this deploy owns. Anything else already in .env.prod is left untouched.
MANAGED_KEYS=(
  LOONTAIL_IMAGE
  POSTGRES_USER
  POSTGRES_DB
  POSTGRES_PASSWORD
  DATABASE_URL
  NETWORK_DOMAIN
  CORS_ALLOWED_ORIGINS
  YGGDRASIL_PUBLIC_URL
  YGGDRASIL_SKIN_DOMAINS
)

LOONTAIL_IMAGE="$IMAGE"
POSTGRES_USER="$DB_USER"
POSTGRES_DB="$DB_NAME"
POSTGRES_PASSWORD="$DB_PASSWORD"
DATABASE_URL="postgres://${DB_USER}:${DB_PASSWORD}@postgres:5432/${DB_NAME}"
# Without this the compose default is CORS_ALLOWED_ORIGINS=* — a permissive
# production origin shipped by the deploy path rather than chosen.
CORS_ALLOWED_ORIGINS="https://${NETWORK_DOMAIN}"
# The signed profile's texture URL must be ABSOLUTE (the game client cannot fetch a
# server-relative path) and its host must be whitelisted in skinDomains, or every
# player silently falls back to the default Steve/Alex skin. Both are derived from
# NETWORK_DOMAIN so a deploy can never ship the path-only compose defaults.
YGGDRASIL_PUBLIC_URL="https://${NETWORK_DOMAIN}/api/yggdrasil"
YGGDRASIL_SKIN_DOMAINS="${NETWORK_DOMAIN},localhost"

# why: this used to `sed -i "s|^${key}=.*|${key}=${value}|"`, which interpolates a
# secret into a sed program — a `|` in the value rewrites the expression (GNU sed's
# `w`/`e` flags can then write files or run commands) and a value ending without a
# newline concatenated onto the previous key, so the key was never applied and the
# deploy silently rolled out the PREVIOUS image. Rebuilding the file instead means
# no value is ever parsed as code, and `printf` always terminates the line.
write_env() {
  local tmp key filter
  tmp="$(mktemp "${DEPLOY_DIR}/.env.prod.XXXXXX")"
  chmod 600 "$tmp"
  filter="$(IFS='|'; echo "${MANAGED_KEYS[*]}")"
  if [ -f .env.prod ]; then
    grep -vE "^[[:space:]]*(${filter})=" .env.prod >> "$tmp" || true
  fi
  for key in "${MANAGED_KEYS[@]}"; do
    printf '%s=%s\n' "$key" "${!key}" >> "$tmp"
  done
  mv "$tmp" .env.prod
  chmod 600 .env.prod
}

write_env

compose() {
  docker compose -f docker-compose.prod.yml --env-file .env.prod "$@"
}

echo "$REGISTRY_TOKEN" | docker login ghcr.io -u "$REGISTRY_USER" --password-stdin
trap 'docker logout ghcr.io >/dev/null 2>&1 || true' EXIT

export LOONTAIL_IMAGE

echo "Pulling $IMAGE ..."
compose pull

echo "Starting containers ..."
compose up -d --remove-orphans

# Confirm the running container really is the digest we shipped, not a leftover
# from a previous `up` that compose decided not to recreate. Compared as image
# IDs, because `.Config.Image` echoes whatever ref was requested rather than the
# content that got resolved.
expected_id="$(docker image inspect --format '{{.Id}}' "$IMAGE" 2>/dev/null || true)"
running_id="$(docker inspect --format '{{.Image}}' loontail-launcher-api 2>/dev/null || true)"
if [ -z "$expected_id" ] || [ "$running_id" != "$expected_id" ]; then
  echo "Running container image is '$running_id', expected '$expected_id' ($IMAGE) — aborting." >&2
  exit 1
fi

# Probe inside the container (the app always listens on 8080 there), so this
# works regardless of which host port the service is published on.
echo "Waiting for /health ..."
for _ in $(seq 1 30); do
  if compose exec -T loontail-launcher-api curl -fsS http://localhost:8080/health >/dev/null 2>&1; then
    echo "Service healthy: $IMAGE"
    docker image prune -f >/dev/null 2>&1 || true
    exit 0
  fi
  sleep 2
done

echo "Service did not become healthy within 60s — rolling deploy failed." >&2
logs="$(compose logs --tail=120 loontail-launcher-api 2>&1 || true)"
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
