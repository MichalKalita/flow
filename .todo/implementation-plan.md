# Implementation plan

## Purpose and decision status

This is the recommended dependency order for the full `.todo` scope. Prioritize reliable system operation, then public access and reusable runtime mechanisms, then applications and integrations. The product goal is reliable, simple software for companies on inexpensive servers, for users without infrastructure expertise, with preserving data ahead of throughput, not Kubernetes-like orchestration or thousands-node scaling. This is a design plan; no feature implementation is claimed by these documents.

Each topic distinguishes requested outcomes, current implementation, recommendations, and open decisions. Answer questions in conversation; record accepted decisions back in the corresponding English document. Unanswered recommendations must not be treated as approved product requirements.

## Confirmed decisions

- Provide a predefined project catalog with actual one-click installation into a usable application.
- A complete built-in email server is required, modeled as a hosted project whose program controls application storage and reactions.
- Email input is an ordinary event, exactly like every other event. Only the source and typed payload differ. A source plugin handles mail-specific protocols, verification, and limits.
- Events are accepted and durably stored before handlers run. All sources use one runtime event lifecycle, dispatcher, permission model, and failure mechanism. Existing synchronous stream automations must be migrated explicitly to this target.
- Failure handling is configurable throughout the system, including retry limits and the terminal choice to retain or delete failed input/work.
- Email receipt limits, including individual message size, must be configurable.
- Each server generates and persists its own cryptographic key material on first startup; no administrator-supplied bootstrap key is required. Project secrets remain manageable through protected administration.
- The normal user experience requires no infrastructure expertise: task-oriented setup, peer connection, backup, and restore flows; advanced technical controls stay optional.
- Good default behavior is a product priority: ordinary use should not require tuning dozens of parameters. All settings have operationally sound defaults and guided configuration. The wizard recommends suitable values using host/workload evidence; it collects required peer/provider details and lets administrators review validated changes.
- Every server is named and unambiguously identified in logs and operational views. Multiple servers sharing an S3 destination retain distinct archive ownership; the exact identifier scheme is still an implementation choice.
- HA uses a primary and standby with reciprocal public-key trust. Only one member needs a reachable peer endpoint; the other initiates an outbound bidirectional channel. Network direction is independent of primary/standby role. Private keys remain on their owning server; successful writes/events require durable replica confirmation; takeover is automatic once safe ownership/durability is established; public-ingress fallback remains open.
- Every HA member runs the same software, hosts every project, and holds full project database replicas. One primary owns active work for the whole deployment. No sharding, per-project placement, or Kubernetes-like orchestration is intended.
- Protected administration may show a masked secret preview with a few leading and trailing characters. Do not send the full stored value to the browser for masking; mask short values fully where necessary.
- Operational logs have one server-wide owner, store, byte target, and retention policy. Projects are filter metadata. Remove oldest chunks as new logs arrive; brief overshoot of a few seconds is allowed while retained local bytes converge to the target.
- Administration shows the timestamp and age of the oldest locally retained log. Optional S3 export is configured for the whole server, with a separate chunk-size/export threshold. Schedule upload as soon as a chunk closes; do not wait for the local target to fill. Each server, including each HA member, logs and exports only for itself.
- Do not lose acknowledged application data: required durable replication precedes successful write/event acknowledgement. If replication cannot confirm durability, preserve consistency rather than silently accepting potentially lossy writes.
- Log archives and backups support ordinary local/remote disks and standard storage protocols through shared adapters, including SMB/NFS, SFTP, FTP/FTPS, WebDAV, and S3.
- Provide configurable backups and a functional restoration path, with defaults and wizard guidance.
- The log explorer reads S3-only history using the same filters. In HA, members return their own local results and the request-receiving server queries S3 and merges one deduplicated response.
- A short final pause of writes to the affected project is acceptable for schema migration cutover. Preparation happens while the old version remains active.

## Invariants for every phase

Preserve central application permissions, project isolation, positive numeric branded IDs, transactional allocation, and committed sequence high-water marks across restart and deletion. Every committed application mutation and its SQLite audit must share a transaction; failed attempts must not leave committed-change audit entries.

Validate complete candidates before replacing active programs/configuration/routes. Failed reloads retain the last working project and preserve data/audit while writing the project error file. Any new cross-project access uses an explicit general configuration mechanism.

Keep operational logs and telemetry outside application databases and exclude credentials, tokens, bodies, and plugin arguments. Keep the protected admin listener separate; its console uses normal application permissions. Maintain bounded memory, disk queues, retained work, and telemetry on a 1 CPU / 2 GB host, while allowing larger hosts to use more resources. No required external monitoring stack.

## First programming work and first usable release

The first implementation is persistent server initialization and settings. Build on the existing Rust runtime rather than creating a new platform. Then deliver a usable path from installation to one everyday catalog application. The full feature sequence below remains the final scope; the catalog must not wait for every advanced integration to be complete.

### First implementation — persistent server state

Add a host-owned state module integrated with runtime startup. Generate and persist a stable instance identity, readable default name, peer identity keys, and local secret-encryption material on first start. Reuse them on restart. Store typed, versioned host/project settings with good defaults and protected masked-secret access. Preserve existing explicit configuration during transition.

Start from `runtime/src/main.rs`, `runtime/src/engine.rs`, `runtime/src/projects.rs`, and `runtime/src/resources.rs`. Existing cryptography and SQLite dependencies are available; this work must not require an external key service. Keep application databases, audit, permissions, and numeric entity allocation unchanged.

The first delivery is complete when concurrent first starts cannot create conflicting identities, interrupted initialization recovers, secrets are encrypted, missing/corrupt existing keys fail safely, and reload/restart preserves the active settings. Add meaningful restart/failure/isolation integration tests and document state location and recovery. Do not claim the setup wizard is complete merely because keys are generated.

### First usable release — delivery order

1. Persistent server state and validated settings, as specified above.
2. Protected first-run enrollment and a short setup wizard with working defaults. Preserve the separate admin listener, do not log bootstrap credentials, and do not expose unauthenticated setup publicly.
3. A real local backup-and-restore path covering settings/keys, project data, audit, IDs, and payloads. Prove restoration in a fresh installation before destructive changes are shipped.
4. A small application frontend with ordinary user login and permission-checked operations, reusing the generic frontend contract.
5. A bundled initial catalog template with one-click installation to a usable screen. Recommend a simple contacts application as the first everyday demonstration; the initial catalog contents remain an open product decision.
6. Verify install, use, restart, backup, and restore end to end on the minimum host. Only then call the first usable release delivered.

Use the current local application authentication as a starting point. This release does not need all external login providers, every storage protocol, or the full email service before users can try an application. It must not introduce shortcuts that bypass permission/audit, produce unrecoverable data, or create separate settings/event engines. All requested HA, protocol, catalog, and email scope remains in the full delivery plan.

### Next reliability release

Deliver the universal durable event contract and full-server primary/standby replication with automatic safe takeover. Expand backup destinations and remote log exploration through the shared storage adapters. Catalog and frontend then expand to additional polished applications. Keep reliability gates explicit and do not label single-server operation as completed HA.

## Recommended sequence

| Step | Deliverable | Prerequisites | Completion evidence |
| --- | --- | --- | --- |
| 1 | [Variables and secrets](variables-and-secrets.md): persistent server name/identity and generated keys, typed declarations, encrypted store, generated defaults, shared settings/default schema, validated live settings | Existing protected admin surface | Server identity, keys, and defaults persist once; masked secret previews expose no full stored values; invalid activation and crash recovery preserve the working configuration |
| 2 | [Runtime logs](runtime%20logs.md): one server log service, approximate byte target, independent chunk size, project-tag filtering/verbosity, oldest age, histogram, admin settings | 1 | Oldest segments are overwritten and retained bytes converge after brief overshoot; oldest-age display and bounded search/histograms work |
| 3 | [Capacity guidance](benchmark.md): shared setup wizard with default/recommended values, passive inventory and opt-in bounded calibration | 1, 2 | Isolated/cancellable diagnostics produce evidence-based suggestions; production data is untouched |
| 4 | [Backup and restoration](backups.md): full deployment snapshot/manifest, local manual capture and validated restore, recovery-key handling | 1, 3; consistent snapshot/activation primitives | A fresh isolated runtime restores all projects/data/audit/IDs/payloads/secrets; failed/corrupt backups cannot replace a working generation |
| 5 | [Local migrations and seeds](migrations.md): migration metadata, candidate validation, recovery, test/production groups | 1, 4; use 3 for storage preflight | Old version runs during preparation; short write pause reconciles changes; failures retain working data/audit; sequences and seed history survive |
| 6 | [HTTPS and routing](https.md): built-in TLS/ACME, explicit domain/path aliases | 1; route candidate validation | Renewal/restart and shared-domain routes work; canonical project paths and protected admin isolation remain intact |
| 7 | [Universal event lifecycle](external-integrations.md#confirmed-unified-event-mechanism): common durable acceptance, dispatch, failures, typed source-plugin API | 1, 2, 5 | Existing streams and plugin sources use the same API and lifecycle; committed input survives failed handlers; retry/retain/delete recover across restart |
| 8 | [HA pairing and coordination](variables-and-secrets.md#ha-scope-and-open-architecture): full-server primary/standby replicas, peer trust over an outbound-capable link, no-loss acknowledgement and promotion/ingress contract | 1, 4, 5, 6, 7; HA-6 ingress decision | Every member has identical software/all projects/full databases; peer keys authenticate mutually; data/audit/IDs/events and whole-server primary ownership survive restart/disconnect/takeover |
| 9 | [External effects and data-only endpoints](external-integrations.md#commit-and-external-effects): typed connector methods, outbox, outcome reconciliation, validated remote results, optional server-log destination export/search and scheduled/remote backup integration | 7; 8 for HA durability; 6 for public callbacks; destination upload/read capability for archival | Local writes/audit/intents are atomic; external ambiguity is explicit; external-only endpoints validate responses; each server exports its own closed chunks immediately; coordinated local/S3 log search and backup jobs stay bounded and recoverable |
| 10 | [Shared accounts](shared%20account.md): explicit trust, central identity project, local extensions | 1, 5, 6, 9 | Consumer profiles and roles remain local; explicit issuer/audience bindings and account lifecycle prevent implicit sharing |
| 11 | [External authentication](external%20auth.md): reusable provider login/linking, multiple providers | 1, 6, 9; 10 for cross-project login | Verified callbacks, safe linking, session/revocation behavior, and generic/named-provider support are covered |
| 12 | [Delegated permissions](subpermissions.md): bounded scopes, expiry, revocation, issuance rights | 5 and stable actor contract; 10 only for shared parents | No escalation across fields, records, plugins, or transports; parent permission loss and expiry affect active access |
| 13 | [Storage destinations and S3](s3.md): standard disk/network archive/backup adapters, S3 signing/listing/upload lifecycle and public URLs by policy | 1, 5, 9 | Required providers pass compatibility checks; incomplete uploads are not ready files; cross-project access is rejected |
| 14 | [Generic frontend](generic%20frontend.md): project-controlled application UI and typed metadata | 6, stable metadata, 11 or another defined application login; 13 for S3 widgets | Enabled/disabled state, login, forms, validation, and permission changes pass E2E without admin privileges |
| 15 | [Project catalog](catalog.md): curated ready-to-use templates and one-click isolated installation | 1, 5, 14; application login; 8 for HA durability | One click opens a usable application with generated settings/credentials and normal owner permissions; failed/retried installs preserve data and replicate safely |
| 16 | [Cross-project migration](migrations.md#cross-project-transfer): explicit mappings, provenance, resumable copy, optional deletion phase | 5, 7, 9 and explicit cross-project configuration | Numeric IDs/references remap correctly; retries deduplicate; partial completion and separately authorized source deletion recover safely |
| 17 | [Complete email server](emails.md): source plugin, configurable receipt, submission/delivery, mailbox management, verification | 1, 2, 5, 6, 7, 9; 10 only if shared identities; 13 optional for object storage | Full receiving/sending/mailbox flows and project-defined handlers operate through universal events within bounded host resources |

This table is the full dependency order; the first usable release above delivers a coherent initial slice of those same features sooner. It is not a claim that all earlier features are hard prerequisites for every later feature. After step 9, S3 and local-account delegation can proceed independently of shared accounts. Cross-project transfer does not require shared login. Standalone external login can precede shared account federation. Email can start after its hard foundations and must not require an S3 service; the complete server remains a later milestone because of protocol and operational scope.

Server identity/key generation belongs to step 1. Protected peer transport can be prepared with step 6; replication/HA coordination in step 8 follows verified backup/migration and common event state; external connectors in step 9 then use that durability boundary. Single-server features remain usable without an HA deployment. Pairing alone is not considered completed high availability.

Certificate renewal can start with a small bounded implementation at step 6, but it must use the shared failure policy and join the common job/event infrastructure when step 7 is delivered. This is a staged integration, not a permanent second scheduler for email or certificates.

## Milestones and exit gates

### M1 — Reliable operating foundation: steps 1–5

Administrators manage settings securely with a shared default/settings schema and wizard that recommends suitable values; one server log service targets a bounded local byte budget with brief overshoot and reports actual oldest-log age; capacity diagnostics are safe; whole-deployment backups restore successfully into a fresh runtime; incompatible schema changes have a validated, recoverable migration path. Demonstrate this on the minimum host, including audit growth and space reserved for migration/backup/event storage. Existing routing, permissions, and rollback remain covered.

### M2 — Secure public hosting: step 6

A configured shared domain routes an account prefix and an application root without ambiguity. Certificates acquire/renew and survive restart. Failed candidates keep active routes intact. HTTP/WebSocket remain project-isolated and admin APIs remain on the protected listener.

### M3 — One event, full-replica reliability, and integration model: steps 7–9

Local sources and plugins share durable acceptance, dispatch, actors, retry limits, and terminal retain/delete. Update existing automation behavior explicitly: handler failure cannot undo a committed source event, but each failed handler attempt rolls back its own application mutations and audit. Verify restart at every transition, quotas, event-chain limits, schema-versioned queued payloads, and handler-version behavior.

Optional server-wide log archival across configured standard storage destinations uses the same connector/job infrastructure with separate host credentials and a bounded local backlog. Chunk size is independent of local retention; each HA member uploads its own chunk immediately after closure and retains its local copy until ordinary cleanup. The configured destination-outage policy and oldest-local-log/export-status UI are verified. The log explorer combines member-local records and archive-only history, including S3, disk/share, and other standard adapters, under identical filters on the request-receiving server, with deduplication, bounded pagination, and explicit partial status. Integrate scheduled backups and remote backup storage with the common job/storage contract. Complete standard storage-adapter coverage and full application S3 operations remain in step 13; a single S3 adapter does not complete the broader destination scope.

A declared external-data-only endpoint authenticates, checks permissions, and validates the remote result without requiring local business entities. Outbox delivery never promises remote rollback or exactly-once execution without destination support.

### M3a — Full-server HA reliability gate: step 8

Full-server primary/standby replication, identical software/all projects/full databases, and single-endpoint peer reachability are confirmed. No loss of acknowledged data and automatic takeover are also confirmed. Resolve HA-6 public-ingress fallback and implement tested ownership/durability checks before claiming full failover. Demonstrate reciprocal peer-key verification, a dial-only member behind NAT, bidirectional traffic over one connection, checkpointed reconnect, and persistence across restart. Verify public-ingress failover separately from synchronization connectivity. Verify complete project/database synchronization, coordinated software/schema generations, and deployment-wide primary/worker ownership under automatic takeover, reconnecting old primaries, disconnect, stale peers, restart, migration, key rotation, and external-effect ambiguity. Acknowledged data/audit and committed sequence/event state must survive primary loss; verify durable replication before acknowledgement and unavailable writes when required replication is down. Test stale-primary rejection and recovery without duplicate mutations/effects. Each server keeps its own operational log retention, chunk threshold, stable archive identity, and export worker; server names/identifiers are visible and distinct across a shared S3 destination; HA must not replicate logs or delegate their export to a peer.

### M4 — Identity and limited access: steps 10–12

A central identity project works with explicit consumer bindings and local extensions. Multiple configured providers can coexist. Delegated credentials are bounded by current parent permissions and revocation. Include all named providers in the delivery backlog; a generic OIDC prototype alone does not finish that topic.

### M5 — Application, catalog, and storage workflows: steps 13–16

Standard storage destinations and an ordinary-user frontend are usable without admin credentials. A predefined catalog creates an isolated ready-to-use project with one click; it does not require source editing or a technical follow-up wizard. Explicit cross-project transfer remaps references, resumes after failures, and records audit locally in each database. Optional source deletion is a separate reviewed operation, not an assumed distributed rollback.

### M6 — Full built-in mail and final capacity validation: step 17

Follow the email document's sub-milestones through receipt, submission/delivery, and complete mailbox management. A receive-only or SMTP-send-only prototype does not satisfy the full email server. Prove that email uses the same event infrastructure as other sources and that project configuration controls storage/handlers and receipt limits. Revalidate the whole system on the minimum host and scaling behavior on a larger host.

## Decision queue

Resolve the earliest choices first, while continuing design work independent of the answer:

1. **CFG-2:** live settings boundary. Good operational defaults and a minimal recommendation wizard are confirmed; select and test ordinary numeric defaults during implementation instead of asking the user to tune every parameter. **CFG-1 is revised: automatic persistent server keys and project secret administration. CFG-3 is answered: allow a masked preview showing a few first and last characters.**
2. **HA-6:** public-ingress fallback. **HA-1–5 are answered: automatic safe takeover; preserve acknowledged data with durable replication before successful write/event acknowledgement; full-server primary/standby, identical software/all projects/full database replicas, only one required reachable peer endpoint, and reciprocal key trust over an outbound-capable channel.**
3. **BKP-3–5:** restore granularity, portable recovery keys, and loss-of-all-members recovery. Schedule/retention mechanics receive tested implementation defaults. Actual backup/restoration support and standard local/network/S3 destinations are confirmed.
4. **LOG-4–5 / CAP-1:** export failures/remote retention and optional active diagnostic safety. Histogram windows and routine capacity/application guidance receive good defaults; do not ask the user to pick low-level tuning values. **LOG-1, LOG-3, and LOG-6–9 are answered: overwrite oldest server-wide chunks toward the byte target, allow brief overshoot, show oldest retained log age, export immediately on configured chunk closure, keep export local to each HA server, identify the originating server in records/archives, and merge peer-local plus configured archive history on the request receiver. Server-wide archival across standard storage destinations is confirmed.**
5. **MIG-2–3:** transfer deletion and seed-triggered events. **MIG-1 is answered: a short final write pause is allowed.**
6. **TLS-1–3:** aliases, wildcard requirements, and domain ownership/configuration.
7. **INT-1–3:** additional event sources, plugin extension trust, and common policy placement/overrides. The universal event/failure model itself is confirmed.
8. **ACC-1–3 / AUTH-1–3 / DEL-1–3:** provisioning, revocation, roles, provider/client priorities, and delegation defaults.
9. **OBJ-1–3 / UI-1–3:** deployment providers, public/multipart objects, frontend exposure/customization/registration.
10. **CAT-1–3:** initial everyday applications, multiple instances, and catalog delivery/updates. Actual one-click use is confirmed.
11. **MAIL-1–3:** mailbox interfaces, raw-message retention, and direct/relay delivery. Complete built-in mail, the common event lifecycle, and configurable limits are already confirmed.

The detailed English questions/recommendations live in their topic files. Ask them in Czech in conversation, record answers, and revise dependencies when a decision changes the implementation contract. Do not ask the user to choose the general implementation order; derive it from system needs and dependencies.

## Verification and delivery workflow

Implement coherent parts and commit each with concrete English messages. Read relevant implementation/tests first and add meaningful integration coverage for behavior changes. Update runtime/language/project documentation when the feature is implemented; `.todo` designs alone do not document a shipped API.

For Rust changes, run in `runtime/` before delivery:

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

For frontend changes, run in `admin-ui/`:

```sh
bun run format
bun run check
bun test src
bun run build
bun run test:e2e
```

Use the real runtime and isolated temporary databases for admin/application E2E. Commit small rebuilt assets in `runtime/src/admin-assets/` alongside frontend source. Do not commit databases, logs, secrets, or build directories.

Capacity and mail interoperability need explicit bounded operational validation in addition to unit/integration tests. Report the workload and environment; green functional tests alone do not prove production capacity or external-service compatibility.

## Implementation progress

Persistent server identity/key generation, authenticated-encrypted settings, masked secrets, project secret references, protected local enrollment, owner password login, and the initial settings UI are implemented. Restart/concurrent initialization/corrupt-key tests and real-runtime listener/authentication tests pass. The first-run browser flow passes through restart; existing dashboard E2E flows pass. Password-encrypted key recovery is implemented and being integrated with complete backup/restore. Catalog installation, application frontend, and full backup/restore remain in progress; HA and subsequent features are not implemented yet.
