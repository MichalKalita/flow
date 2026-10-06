# Migrations and seeds

## Requested outcome

Migrate incompatible schemas and data before activating the new program, while the previous program continues operating during preparation. Support explicit entity/data transfer between projects. Distinguish test seeds from production seeds and prevent repeated insertion.

## Current implementation

Reload accepts compatible additions and rejects entity/field removal or changes to stored types and constraints. Candidate schema changes and seed inserts are transactional; failed reloads retain the last working program, preserve data/audit, and write the project error file. The cached validated program supports recovery after restart.

Seeds have an entity/numeric-ID ledger. A seed is initialized once; deleting it does not recreate it on reload. Explicit IDs are numeric and allocation remains above existing values. See [reload and initialization](../runtime/src/engine.rs), [hosted loading](../runtime/src/projects.rs), and [reload integration tests](../runtime/tests/projects.rs).

## Recovery prerequisite

Provide a verified [backup and restore workflow](backups.md) before activating destructive schema/data changes. A replica is not the historical restore point for a migration mistake. Check a usable backup and sufficient staging space; backup file creation without demonstrated restoration is insufficient.

## Local migration options

1. Pause one project's work and migrate in place transactionally. Simple and safe for small changes, but large migrations interrupt service.
2. Prepare a candidate database from a consistent snapshot, transform and validate it while the old version remains active, then reconcile writes and cut over. Recommend this for changes requiring lengthy preparation.
3. Use expand/backfill/contract changes against a schema compatible with both programs. Useful for some changes, but requires explicit compatibility rules and more stages.

Do not switch to a stale snapshot after the old program has accepted new writes. The user has confirmed that a short final pause of project writes is acceptable. Prepare while the old version runs, then pause writes, reconcile changes, validate, and cut over. Fully uninterrupted writes are not required for the initial migration design.

A consistent SQLite snapshot should use a supported backup mechanism, not copying only a live main database file. See [SQLite's backup API](https://www.sqlite.org/backup.html).

## Proposed activation protocol

Declare source and target schema versions, an immutable migration identifier/checksum, bounded transformations, and pre/postconditions. Preview changed entities, references, expected storage, and incompatibilities. Validate the complete candidate program and migrated values, uniqueness, references, ID sequences, and schema before activation.

Keep the old database/program authoritative until cutover. Account for writes during preparation; pause/drain project handlers and streaming writers consistently when required. Commit each authoritative application mutation together with its audit. Candidate preparation must not manufacture committed business-change audit entries in the active store. Preserve audit lineage and sequence high-water marks in the candidate, including sequences above deleted IDs.

Coordinate database, cached program, configuration, and routing activation with a durable recovery state. Recover to one known authoritative generation after a crash. On failure, retain the previous working project and its data/audit and write the project error file. Once the new version has accepted writes, switching back is not equivalent to restoring an old backup; require an explicit reverse migration or recovery procedure.

## HA coordination

The confirmed HA topology is a full-server primary and standby with reciprocal key trust and only one required reachable peer endpoint. Every member runs the same software and holds all projects with complete database replicas. Replication acknowledgement must preserve confirmed data without loss; promotion details remain open. Migration activation coordinates with the deployment primary and replica schema/program versions. A replica must not resume writes or dispatch events using incompatible schema or stale committed sequence/event state. Apply the confirmed short final write pause consistently to the affected project across its participating replicas.

Full-server replication must preserve data and audit together and retain migration recovery state. Standbys receive the same project/migration generation and must not resume as a separately writable variant. Coordinate software upgrades so participating HA members run the required same software. Keep original server identity on replicated audit entries; label migration diagnostics with the executing server. Keep the original server identity on replicated audit entries; identify the executing server on migration diagnostics without rewriting mutation provenance. It is distinct from transferring entities between separate projects. See [HA scope](variables-and-secrets.md#ha-scope-and-open-architecture).

## Cross-project transfer

Use an explicit source/target mapping and a versioned transfer job. Export only authorized fields and dependent records. Allocate target-local numeric IDs transactionally and maintain provenance separately as source project, entity, numeric source ID, and transfer version. Rewrite references through the mapping; never assume identical numeric IDs represent shared identity.

Recommend copy-and-validate first, followed by optional source removal as a separate authorized phase. Different databases do not provide one automatic atomic transaction. Persist checkpoints and deduplicate imports so retries do not duplicate data; audit target writes and any source deletions in their respective transactions. Preview reference closure, conflicts, streams/blobs, and event behavior before execution.

## Seeds

Introduce explicit production/bootstrap and test/demo seed groups. Recommend excluding test seeds from production by default. Preserve once-only behavior; changes to bootstrap data should be versioned migrations rather than silent reseeding.

Retain fixed positive numeric IDs where stable bootstrap references require them. An existing row with that ID is not permission to overwrite it. Recommend insert-once with conflict reporting; intentional updates use a migration with preconditions. Advance sequences transactionally and never reuse committed allocations after deletion.

Seed-generated events, if enabled, must use the same committed-event lifecycle as all other sources. Recommend no seed-triggered business events by default, with explicit opt-in and deduplication. This choice is not confirmed yet.

## Confirmed decision

- **MIG-1:** A short final pause of project writes is acceptable. The old version remains available during preparation; pause only the affected project for final reconciliation and activation.

## Open decisions

- **MIG-2:** Should transfer remove source data automatically? Recommend copying first and making deletion a separate explicit phase.
- **MIG-3:** Should seeds trigger ordinary project events? Recommend disabled by default, with explicit opt-in using the universal event mechanism.

Answers: Pending conversation.

## Dependencies and verification

Local migration metadata and recovery precede [the unified event lifecycle](external-integrations.md). Transfers can follow the common job/plugin contract and an explicit general cross-project configuration model; they do not require shared accounts.

Verify incompatible-schema rejection, candidate rollback, writes during preparation, crash at each cutover stage, audit lineage, deleted-ID sequence preservation, seed conflicts/restarts, transfer remapping, duplicate retries, and partial transfer recovery. Use isolated databases and include active HTTP/MQTT/WebSocket work during migration tests.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Define defaults for configurable behavior and expose suitable-value recommendations through the wizard. Collect required external credentials, trust, destinations, or project policy explicitly; keep optional capabilities inactive until valid configuration exists. Numeric values and unconfirmed policy recommendations in this document remain proposals.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.

## Delivered seed groups

Production/bootstrap and opt-in test/demo groups are implemented. Hosted manifests select a project-local boolean profile; both groups are statically validated. Duplicate seed IDs are rejected, first-use collisions reject activation, and initialization/audit/ledger writes are transactional. Disabled groups consume no numeric allocations. Restart/deletion/profile-change tests preserve once-only behavior and committed allocation boundaries. Versioned incompatible data/schema migrations and cross-project transfers remain outstanding.
