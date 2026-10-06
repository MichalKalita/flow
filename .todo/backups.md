# Configurable backups and restoration

## Confirmed outcome

The server must support configurable backups and restoration from those backups. This is part of making Flow reliable and easy to operate for companies on inexpensive servers. A replica is not a substitute for a recoverable historical backup: accidental application changes also reach full replicas.

All settings need defaults and setup-wizard guidance. Expose backup destination, schedule, retention, storage/resource limits, status, and restoration through protected administration, with an offline recovery path if the runtime cannot start.

## Current implementation

Projects use SQLite with WAL, cached validated programs, separate application audit, and local operational/telemetry files. There is no existing configurable whole-deployment backup/restore workflow. See [runtime initialization](../runtime/src/engine.rs), [project loading](../runtime/src/projects.rs), and [project recovery tests](../runtime/tests/projects.rs).

## Implementation options and recommendation

1. A full versioned deployment snapshot. Recommend this first for understandable backup and restore behavior; compression and bounded streaming can control cost.
2. Full snapshots plus incremental history. Useful later if measured backup costs require it, but not a prerequisite for the simple initial design.
3. Storage-provider snapshots as an optional additional mechanism. Do not require a particular cloud or platform to restore Flow.

Support the common configurable [storage destinations](s3.md#confirmed-general-archive-and-backup-destinations): local disks, mounted remote disks, SMB/NFS shares, SFTP, FTP/FTPS, WebDAV, and S3-compatible storage. The user confirms ordinary standard-protocol destinations, not only S3. Local staging/spool must be bounded. A backup becomes available for restoration only when all required parts and its integrity/manifest checks are complete; failed attempts must not look like usable backups.

Use a supported SQLite snapshot/backup mechanism rather than copying a live main database file alone. See [SQLite backup API](https://www.sqlite.org/backup.html). Capture a validated deployment generation and define a consistent point across its project databases, program/configuration, and durable jobs. A coordinated short write pause is a possible implementation option, not an assumed instantaneous snapshot across independent databases.

## Backup contents and ownership

A complete backup includes all project programs/manifests, schemas, complete databases, mutation audit, numeric ID sequence high-water marks, seed/migration metadata, blobs and other locally stored business payloads, encrypted project configuration/secrets, and durable event/effect state. Include software/data-format compatibility and the required restore instructions in a versioned manifest.

Protect recoverable encryption material as part of the recovery design, including a separately protected recovery package or portable encrypted backup format. A dump that cannot decrypt its restored project secrets is not a successful backup. Keep archive credentials/private keys out of logs and normal manifest previews. The exact recovery-key packaging still needs a decision.

HA members run the same software and hold all project data. Recommend one scheduled deployment-backup owner, normally the primary, to avoid duplicate expensive captures; failover ownership follows the same simple primary/standby model. This is separate from log archival, where every server logs and uploads only for itself.

Server names/identities, peer trust/private keys, and deployment settings need an explicit restore choice: restore the same instance or create a replacement instance and re-pair it. Do not accidentally start two active members with the same identity. Operational log history remains in the local/configured-archive logging lifecycle; restoring a business backup must not overwrite or relabel another server's logs.

## Restoration workflow

1. Choose a completed backup and show its capture time, deployment/software version, projects, size, and verification state.
2. Restore into isolated staging with bounded disk/memory use. Verify checksums, format/version compatibility, SQLite integrity, types/references, schema/program compatibility, decryption, and all required payloads.
3. Show the expected replacement/loss of state after the backup point. Preserve a recovery copy of the current working deployment when possible. Do not replace active databases merely because files downloaded successfully.
4. Coordinate a maintenance boundary across the deployment, prevent competing primary writes/job dispatch, and activate a fully validated generation with crash-recoverable state. Rebuild/reseed standby replicas from that authoritative restored generation; an old peer must not overwrite it on reconnect.
5. Resume application work only after the configured validation and external-effect recovery procedure. A restore failure retains the previous usable state whenever it exists and records a protected recovery error.

Preserve audit lineage and known committed allocation high-water marks from the active deployment/peers before replacement. Restoring an older database must not silently reuse IDs allocated after its capture. If all newer state is lost and the remaining backup cannot establish a safe allocation boundary, require an explicit recovery strategy before enabling new allocations; do not claim ordinary restore magically preserves unknown lost history.

Restoring SQLite does not undo emails, payments, uploads, or other external effects already performed after the snapshot. Recommend pausing effect dispatch until completion state/idempotency identifiers are reconciled or reviewed, so stale queued work is not blindly executed again. Recovery uses the common event/failure contract, not a separate backup-only event engine.

## No-data-loss recovery requirement

Reliability and preserving data are the overriding product goals. HA must preserve acknowledged writes through required durable replication; a periodic snapshot alone cannot reconstruct committed changes after its capture point. If disaster recovery must survive loss of all HA members without losing acknowledged data, include a durable off-server change history/continuous backup and the necessary acknowledgement rule. Do not describe a daily full snapshot as meeting that recovery objective by itself.

Historical point-in-time restoration is an intentional choice to revert business state, distinct from no-loss recovery of the latest committed state. Preview that distinction explicitly. Preserve known committed ID allocation boundaries and audit/effect lineage when restoring history. Establish safe recovery from snapshots plus subsequent durable history and test actual replay before claiming no-loss disaster recovery.

The final failure domain and off-server durability requirement still need confirmation. Full snapshots remain useful baseline artifacts, but must not be substituted for the user's no-data-loss objective.

## Scheduling, retention, and administration

Use the common bounded job mechanism for scheduled backups and retry/terminal policy. Configure retained count/age and an independent space allowance; backup cleanup never deletes application audit or the last usable backup due to a partial new attempt. Show last completed backup, failures, next run, destination, oldest recoverable point, and restore-test status.

Provide a guided restore and a documented minimal offline recovery command. Protect destructive activation with an explicit review of the selected concrete backup and affected deployment; a read-only restore validation is independently usable. The wizard should explain capture time and recovery consequences in plain language.

## Open decisions

- **BKP-1 (confirmed):** Support local and remote disks/shares plus common standard storage protocols through the shared destination adapters. The wizard collects destination facts and recommends an off-server copy; do not ask the user to choose protocol implementation details.
- **BKP-2 (implementation default):** Provide a tested, storage-aware schedule/retention preset instead of requiring users to tune backup mechanics. Periodic snapshots can supplement continuous history where no-loss disaster recovery is required; a daily snapshot alone must not be presented as preserving all later acknowledged data. Let the wizard explain the recovery coverage and collect required destination/recovery details.
- **BKP-3:** Should the first restore flow replace the whole deployment or also restore individual projects? Recommend whole-deployment recovery first, with explicit single-project restore later and no implicit merging.
- **BKP-4:** How should backup recovery keys be supplied on a replacement server? Recommend a portable encrypted backup/recovery package that the administrator can safely retain independently of the failed host.
- **BKP-5:** Must recovery preserve all acknowledged data even if all HA members are lost? Recommend defining this explicitly; it requires off-server durable change history in addition to snapshots.

Answers: Pending conversation. Configurable backups and a functional restore path are confirmed; standard local/network/S3 destination support is confirmed; numeric defaults, key packaging, and restore granularity still need design.

## Dependencies and verification

Depends on [settings/key recovery](variables-and-secrets.md), [consistent snapshot and activation](migrations.md), and resource headroom. A local manual backup/restore validation should precede risky schema activation. Scheduling integrates with [common events/jobs](external-integrations.md); local/remote backup targets share the [destination adapters](s3.md). Full HA replication builds on a verified whole-deployment snapshot/restore format.

Test restoration into a fresh isolated directory/runtime, not just backup file creation. Cover multiple projects, matching audit/data, numeric references/sequences, deleted IDs, seeds, blobs, secrets, pending/completed effects, corruption/missing parts, disk-full/interrupted capture, failed activation, identity replacement, stale standby reconnect, and repeated scheduled-job ownership. Confirm resource limits on the minimum host and perform an actual recovery drill.

## Defaults and setup guidance

Use the shared [settings/defaults and setup-wizard contract](variables-and-secrets.md#confirmed-defaults-and-guided-setup). Recommend schedule, retention, destination, and disk headroom from host conditions, while requiring explicit recovery material/destination credentials. Do not silently enable remote uploads to an invented destination.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
