#!/bin/sh
set -eu
umask 077

project_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
cd "$project_dir"

if [ ! -f .env ]; then
    printf '%s\n' 'Missing .env. Copy .env.example to .env and configure FLOW_ADMIN_TOKEN.' >&2
    exit 1
fi

# Load trusted local configuration and export it to the runtime.
set -a
. ./.env
set +a

: "${FLOW_ADMIN_TOKEN:=}"
if [ "${#FLOW_ADMIN_TOKEN}" -lt 32 ]; then
    printf '%s\n' 'FLOW_ADMIN_TOKEN must contain at least 32 bytes.' >&2
    exit 1
fi

: "${FLOW_APPLICATION:=runtime/application.flow}"
: "${FLOW_DATABASE:=data/flow.sqlite}"
: "${FLOW_HTTP_BIND:=127.0.0.1:8080}"
: "${FLOW_MQTT_BIND:=127.0.0.1:1883}"
: "${FLOW_ADMIN_BIND:=127.0.0.1:9090}"
: "${FLOW_OBSERVABILITY_DIR:=data/observability}"
export FLOW_ADMIN_BIND FLOW_OBSERVABILITY_DIR

mkdir -p -- "$(dirname -- "$FLOW_DATABASE")" "$FLOW_OBSERVABILITY_DIR"

# A single build job keeps compilation memory bounded on the target VPS.
exec cargo run --release --locked --jobs 1 --manifest-path "$project_dir/runtime/Cargo.toml" -- \
    "$FLOW_APPLICATION" "$FLOW_DATABASE" "$FLOW_HTTP_BIND" "$FLOW_MQTT_BIND"
