#!/bin/sh
set -eu
umask 077

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$project_dir"

if [ ! -f .env ]; then
    cp .env.example .env
fi

# Load trusted local configuration and export it to the runtime.
set -a
. ./.env
set +a

: "${FLOW_ADMIN_TOKEN:=}"
if [ -n "$FLOW_ADMIN_TOKEN" ] && [ "${#FLOW_ADMIN_TOKEN}" -lt 32 ]; then
    printf '%s\n' 'FLOW_ADMIN_TOKEN must contain at least 32 bytes.' >&2
    exit 1
fi
if [ -z "$FLOW_ADMIN_TOKEN" ]; then
    unset FLOW_ADMIN_TOKEN
fi

: "${FLOW_APPLICATION:=projects}"
: "${FLOW_DATABASE:=data/projects}"
: "${FLOW_HTTP_BIND:=0.0.0.0:80}"
: "${FLOW_MQTT_BIND:=127.0.0.1:1883}"
: "${FLOW_ADMIN_BIND:=127.0.0.1:9090}"
: "${FLOW_OBSERVABILITY_DIR:=data/observability}"
export FLOW_ADMIN_BIND FLOW_OBSERVABILITY_DIR

mkdir -p -- "$(dirname -- "$FLOW_DATABASE")" "$FLOW_OBSERVABILITY_DIR"

if ! command -v bun >/dev/null 2>&1; then
    printf '%s\n' 'bun is required to build the admin UI. Install Bun and retry.' >&2
    exit 1
fi

# Embed fresh control-plane assets before the Rust binary is compiled.
(
    cd "$project_dir/admin-ui"
    bun install --frozen-lockfile
    bun run build
)

# A single build job keeps compilation memory bounded on the target VPS.
exec cargo run --release --locked --jobs 1 --manifest-path "$project_dir/runtime/Cargo.toml" -- \
    "$FLOW_APPLICATION" "$FLOW_DATABASE" "$FLOW_HTTP_BIND" "$FLOW_MQTT_BIND"
