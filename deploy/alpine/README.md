# Alpine Linux deployment

The Raspberry Pi deployment runs the ARM64 runtime under the unprivileged `flow`
account, supervised by OpenRC. The operating system uses a persistent ext4 root
filesystem in Alpine sys mode. Application data does not require `lbu commit`.

## Installed layout

- `/data/flow/releases/<release>/flow-runtime`: release binary and `release.json`.
- `/data/flow/releases/<release>/projects/`: hosted Flow sources and manifests.
- `/data/flow/current`: selected release symlink.
- `/data/flow/data/projects/`: separate SQLite databases and observability stores.
- `/data/flow/data/observability/`: shared host observability store.
- `/etc/flow/runtime.env`: credentials and configuration, mode `0640`, owner
  `root:flow`; its parent directory is mode `0750`, owner `root:flow`.
- `/usr/local/bin/flow-start`: installed copy of [flow-start](flow-start).
- `/etc/init.d/flow`: installed copy of [flow.openrc](flow.openrc).

Keep credentials out of release directories and Git. The deployment uses the
existing local credentials from `.env`, followed by these deployment overrides:

```sh
FLOW_APPLICATION='/data/flow/current/projects'
FLOW_DATABASE='/data/flow/data/projects'
FLOW_OBSERVABILITY_DIR='/data/flow/data/observability'
FLOW_HTTP_BIND='0.0.0.0:80'
FLOW_MQTT_BIND='127.0.0.1:1883'
FLOW_ADMIN_BIND='0.0.0.0:9090'
FLOW_ADMIN_ALLOW_REMOTE=1
FLOW_TOKIO_WORKERS=2
FLOW_TOKIO_BLOCKING=2
FLOW_HTTP_ADMISSION=64
```

HTTP is available on the LAN on port 80, and the separate admin listener is
available on port 9090 with explicit network opt-in. MQTT listens only on loopback.
The admin API still requires `FLOW_ADMIN_TOKEN`. Each hosted project retains its own database,
actors, permissions, and observability files.

## Building ARM64 on a development machine

Run from the repository root with Docker available:

```sh
flow_build_dir=$(mktemp -d)
docker run --rm --platform linux/arm64 \
  -v "$PWD:/source:ro" -v "$flow_build_dir:/work" alpine:3.24.2 \
  sh -ec 'apk add --no-cache rust cargo build-base; cargo build --manifest-path /source/runtime/Cargo.toml --release --locked --jobs 4 --target-dir /work/target'
```

The binary is `$flow_build_dir/target/release/flow-runtime`. The target Pi requires
Alpine ARM64 and the `libgcc` runtime package. SQLite and the dashboard assets are
embedded; Bun and Rust are unnecessary on the Pi. Rebuild the versioned admin assets
and run the prescribed frontend checks when changing frontend source.

Validate every hosted program with `flow-runtime --check <application.flow>` before
selecting a new release. Preserve `/data/flow/data` when replacing a release.

Install `libcap-utils` on the Pi and grant the root-owned executable permission to
bind port 80 before starting each new release:

```sh
setcap cap_net_bind_service=ep /data/flow/releases/RELEASE/flow-runtime
```

The service account must not be able to modify this executable. Reapply the
capability after replacing the binary; the service continues to run as `flow`.

## Operating the service

On the Pi:

```sh
rc-service flow status
rc-service flow restart
rc-update show default
```

The service starts at boot, uses bounded automatic respawn, and sends startup/error
output to Alpine syslog, whose default rotation retains two 200 KiB files. Runtime
application logs and telemetry use their own bounded stores outside SQLite.
OpenRC stops the runtime with SIGINT so it can flush observability and shut down.

Open `http://PI_ADDRESS:9090` on the local network and use the configured admin
token. An SSH tunnel remains available for loopback access from a development machine:

```sh
ssh -N -L 127.0.0.1:19090:127.0.0.1:9090 root@PI_ADDRESS
```

Open `http://127.0.0.1:19090` and use the configured admin token. The Pi's SSH
configuration permits local forwarding only to `127.0.0.1:9090`, retains key-only
authentication. The admin API requires its token through either access path.

## Deployment verification

The initial deployment passed `cargo fmt --check`, Clippy with warnings denied,
and all 56 Rust tests. The ARM64 binary validated all 13 hosted programs. Real
deployment checks cover LAN HTTP, numeric JSON IDs, protected admin authentication,
project startup, unprivileged execution, persistent SQLite files, and restart.

The Pi has 1 GB RAM, below the project's documented 2 GB production baseline.
Measure the intended workload before treating this deployment as a capacity guarantee.
