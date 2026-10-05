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

## Updating a release

Updates replace the selected release directory. Keep `/data/flow/data` and
`/etc/flow/runtime.env`; they hold the persistent databases, observability,
credentials, and current listener/capacity settings. Do not run `start.sh` on the Pi.

1. Run the required Rust checks in `runtime/`: `cargo fmt --check`,
   `cargo clippy --all-targets -- -D warnings`, and `cargo test`. If frontend source
   changed, rebuild the admin assets and run the required frontend checks first.
2. Build the ARM64 release using the Docker command above. Package the binary and
   hosted programs from the same source revision. From the repository root:

   ```sh
   flow_pi=root@192.168.0.115
   flow_release=$(date -u +%Y%m%dT%H%M%SZ)-$(git rev-parse --short=12 HEAD)
   flow_revision=$(git rev-parse HEAD)
   mkdir -p "$flow_build_dir/package"
   cp "$flow_build_dir/target/release/flow-runtime" "$flow_build_dir/package/"
   cp -R projects "$flow_build_dir/package/"
   printf '{"release":"%s","source_revision":"%s","target":"aarch64-unknown-linux-musl"}\n' \
     "$flow_release" "$flow_revision" >"$flow_build_dir/package/release.json"
   COPYFILE_DISABLE=1 tar -czf "$flow_build_dir/release.tar.gz" -C "$flow_build_dir/package" .
   scp "$flow_build_dir/release.tar.gz" "$flow_pi:/tmp/flow-release-$flow_release.tar.gz"
   printf 'Release to install: %s\n' "$flow_release"
   ```

   This packages the local hosted programs. Include any intended changes made
   directly to the deployed programs before packaging. Credentials are not bundled.

3. Connect to the Pi as root. Set `flow_release` to the exact value printed above,
   then prepare and validate the new release while the old service continues running:

   ```sh
   set -eu
   flow_release='REPLACE_WITH_PRINTED_RELEASE'
   flow_next="/data/flow/releases/$flow_release"
   test ! -e "$flow_next"
   mkdir "$flow_next"
   tar -xzf "/tmp/flow-release-$flow_release.tar.gz" -C "$flow_next"
   chown root:root "$flow_next/flow-runtime"
   chmod 755 "$flow_next/flow-runtime"
   setcap cap_net_bind_service=ep "$flow_next/flow-runtime"
   chown -R flow:flow "$flow_next/projects"
   for flow_program in "$flow_next"/projects/*/application.flow; do
     su -s /bin/sh flow -c "$flow_next/flow-runtime --check $flow_program"
   done
   ```

4. Stop the runtime cleanly and snapshot its data before the new version can write
   to it. Keep the previous release path for rollback:

   ```sh
   flow_previous=$(readlink /data/flow/current)
   flow_backup="/data/flow/backups/before-$flow_release"
   mkdir -p "$flow_backup"
   chmod 700 /data/flow/backups "$flow_backup"
   printf '%s\n' "$flow_previous" >"$flow_backup/previous-release"
   cp /etc/flow/runtime.env "$flow_backup/runtime.env"
   chmod 600 "$flow_backup/runtime.env"
   rc-service flow stop
   tar -czf "$flow_backup/data.tar.gz" -C /data/flow data
   ln -s "$flow_next" /data/flow/current.next
   mv -Tf /data/flow/current.next /data/flow/current
   rc-service flow start
   ```

5. Run `rc-service flow status`, open the admin dashboard, and verify that every
   intended project is `running`. Check an application endpoint such as
   `http://192.168.0.115/demo/api/products`. Parser validation and a successful
   supervisor start alone do not verify compatibility with existing databases.
   Keep the previous release and backup until the new version has been validated.

For a binary-only rollback with unchanged database schemas, run on the Pi:

```sh
rc-service flow stop
flow_previous=$(cat "$flow_backup/previous-release")
ln -s "$flow_previous" /data/flow/current.rollback
mv -Tf /data/flow/current.rollback /data/flow/current
rc-service flow start
```

If an update changed schemas or data incompatibly, switching the binary alone is
insufficient. Plan a compatible migration or restore the stopped-service snapshot;
restoring that snapshot discards writes made after the backup. Never extract a
database backup over a running service. After a successful update, remove its
temporary archive from `/tmp` and retain only the release/backup history you need.

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
