# Hosted projects

Run `./start.sh` from the repository root. Configure credentials in the root
`.env`; `.env.example` documents the settings. HTTP uses `0.0.0.0:80`, MQTT
uses `127.0.0.1:1883`, and the protected admin uses `127.0.0.1:9090`.
For an unprivileged development listener set `FLOW_HTTP_BIND='127.0.0.1:8080'`.
HTTPS and automatic certificates are deferred.

Every immediate subdirectory is a project. Names use 1–64 ASCII letters, digits,
underscores or hyphens; `all` and `system` are reserved. Put its program in
`application.flow`. The sample apps are folders here: `bookstore`, `clinic`,
`delivery`, `demo`, `gym`, `hotel`, `jobs`, `library`, `restaurant`,
`rideshare`, `school`, `social-network` and `tracker`.
The folder name is its HTTP base path: `demo/api/products`, for example, is served
at `http://localhost/demo/api/products`. `bookstore/api/books` is
`http://localhost/bookstore/api/books`. WebSocket paths receive the same prefix.
File URLs produced by the runtime also include the project prefix. MQTT uses one
listener: CONNECT with username `demo/device` or `demo/user` and the corresponding
adapter credential as password. Topics are local to the selected project.

An optional `project.json` maps JWT aliases and event actors to environment
variable names, without storing secret values in the source folder:

```json
{
  "jwt_secrets": { "user": "FLOW_JWT_SECRET" },
  "event_credentials": { "service:1": "FLOW_AUTOMATION_KEY" }
}
```

Use separate environment variables for different projects when isolation of
signing keys is required. The legacy global JWT setting is a fallback only for
projects declaring the `user` adapter. Projects have separate databases, logs,
metrics, actors and permissions. Shared entities or identities are not implemented;
any future sharing must be configured explicitly, independently of local IDs.

The watcher checks modification time and size every two seconds, avoiding a
filesystem watcher dependency and repeated source parsing. Both the Flow source
and project manifest are watched. New project folders are picked up automatically.
A failed initial load leaves other projects running. A failed reload retains the
last valid program and writes `<folder-name>-error.txt` next to the project folder.
The dashboard shows failed and stale projects; a successful reload clears the error.

Reloads permit operation/permission changes, new entities and optional fields on
existing entities. Removing entities or fields and changing existing types or
constraints require an explicit migration and are rejected. Schema changes and
seed inserts run transactionally. Seed IDs are initialized once; deleting a seeded
record through the editor does not recreate it on reload or restart. Validated
programs and environment-variable mappings are cached in the data directory to
retain the last working version even across restart with invalid source files.
Changing environment variables requires restarting the process.

Data is stored under `data/projects/<name>.sqlite`; observability files are stored
under `data/projects/<name>-observability/`. The shared host observer uses
`FLOW_OBSERVABILITY_DIR`. Existing legacy databases are neither migrated nor deleted.
Removing a folder removes it from new HTTP/admin/MQTT routing and preserves its
stored data. Existing streaming connections hold their project runtime until they
close; disconnect clients before decommissioning a project permanently.

At most 32 projects are active, with source files bounded to 4 MiB and manifests
to 64 KiB. Each project has bounded log buffers and telemetry history. Default HTTP
admission is at least 16 concurrent application requests and scales with CPU and
RAM (16 per CPU, capped at 16 per 2 GiB). Admin admits four, and MQTT admits 128
connections across projects. Tokio workers follow CPU count. No external
monitoring service or exporter is required.
Set `FLOW_HTTP_ADMISSION=unlimited` to disable application HTTP admission limits;
numeric values configure a fixed limit, and the default keeps host-based sizing.

In the admin sidebar, select an individual project for its endpoints, archived
logs, audit, JWT issuer, HTTP console and database editor, or select Entire system
for combined metrics, connection counts, resource totals and cross-project audit.
System log browsing shows recent bounded buffers; select a project to search its
disk history. Process memory and CPU belong to the shared process and are counted
once; database/log sizes and connections are summed across projects.

The database editor lists application entities, searches stored fields and pages
by numeric ID. A privileged admin can create, update and delete records without
application permissions. Types, references and atomic audit are still enforced.
Updates/deletes require the record version returned by the browser, so concurrent
changes return a conflict rather than overwrite newer data. The console retains
normal application authentication and permissions. Internal runtime tables and
arbitrary SQL are not exposed. Pages contain at most 50 records; individual records
above 256 KiB are shown as oversized and cannot be edited through this browser.
