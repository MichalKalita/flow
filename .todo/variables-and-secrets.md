# Variables and secrets

## Requested outcome

Projects declare typed variables and secrets, including persistent generated defaults. Administrators can change project settings while the server is running, without SSH. Secrets must not be stored as plaintext.

## Current implementation

`project.json` maps JWT adapters and event actors to environment variable names. The hosted loader reads those values into runtime configuration. There is no general variable declaration, encrypted secret store, or settings editor. Changing process environment values requires a restart. See [project configuration](../projects/README.md) and [the loader](../runtime/src/projects.rs).

## Implementation options and recommendation

1. Extend environment-only configuration. This is simple, but cannot satisfy persistent generated defaults and live administration on its own.
2. Add a protected, versioned configuration store with encrypted secrets and keep environment references as an import/override option. Recommend this approach.
3. Support an external secret manager through an optional adapter later. It must not become required infrastructure on the minimum host.

Keep project namespaces isolated. Separate application settings from host controls such as worker counts and global disk budgets. On the first start, generate the server's own cryptographic key material using cryptographic randomness. Persist it with restricted access and reuse it on subsequent starts; do not generate replacement keys on every restart. Keep private key material separate from ordinary project configuration. Encryption, key rotation, and recovery must use reviewed cryptographic APIs rather than custom encryption.

## Confirmed defaults and guided setup

All server settings have good operational defaults, and a setup/configuration wizard helps users establish the deployment. Good default behavior is a product priority, not merely the existence of a default value. The intended user has no infrastructure expertise. Routine use must not require tuning dozens of technical parameters, editing configuration files, choosing cryptographic algorithms, or understanding replication internals. Advanced settings remain optional, outside the normal setup flow. This includes server naming/identity, local log retention and chunk size, optional disk/network/S3 archival and archive search, backup/restore policy, resource limits, peer/HA settings, and enabled feature configuration. Project settings participate through their declared types, defaults, constraints, and required inputs.

Provide a valid standalone starting configuration. Generate server identity and key material automatically. Optional external capabilities can default to disabled/unconfigured until required peer addresses, public keys, service destinations, or credentials are supplied; never invent valid external credentials or trusted peers. Choose and verify numeric defaults during implementation; do not turn every tuning parameter into a required user question. Adapt resource defaults to detected CPU, memory, and available storage, with reliability and durability ahead of peak throughput.

Use task-oriented flows such as set up the server, connect a standby, select a backup destination, and restore data. The wizard asks only for necessary deployment choices and external facts such as public hostname, peer endpoint/key, backup destination, and credentials. Keep advanced tuning optional. Show the current/default value, recommended value, and a short reason when relevant. Derive resource/storage recommendations from detected CPU, memory, free space, and optional workload measurements. Offer ordinary safe presets where the workload is unknown, labeled as estimates. The administrator can accept or change recommendations within validated limits.

Use one versioned typed settings/defaults contract for startup, persistent configuration, admin forms, and the wizard. Initialize persistent generated defaults once rather than recomputing secrets/identity on each wizard run. Record explicit overrides so routine restart or re-running the wizard does not silently replace them. Validate the complete candidate and apply supported settings live; clearly identify any change requiring restart.

## Proposed declaration and lifecycle

The original idea `[secret TOKEN [default [randomString [length 20]]]]` is illustrative syntax, not supported Flow syntax. Declare type, required/optional status, and default generation explicitly. Generate a missing default once using cryptographic randomness, persist it, and reuse it across reloads and restarts. Choose entropy by intended use; a generic length of 20 is not automatically suitable for every signing key.

Validate a complete candidate before activation. Failed candidate validation must preserve the active configuration and last working program. Concurrent initialization must not generate multiple committed defaults. Coordinate persisted configuration versions, program activation, and recovery so a crash cannot expose a half-applied candidate.

Represent a secret as an opaque capability usable only by authorized native/plugin consumers. Do not expose the full value through ordinary query projection, errors, logs, cached source, or admin read APIs. The protected settings editor supports replacement and a masked preview showing a few leading and trailing characters, with the middle hidden. Generate the preview on the server; do not send the full secret to the browser merely to mask it there. Fully mask short values when showing both ends would reveal the entire value or too much of it. Keep previews out of operational logs and ordinary application APIs. Key rotation requires an explicit overlap/revocation policy for credentials already in use.

## Confirmed decisions

- **CFG-1 (revised):** The server generates its own encryption key material on first startup and persists it for later starts. An administrator-supplied bootstrap key is not required. Manage ordinary project secrets through protected administration without SSH. Preserve protected key backups for recovery.

- **CFG-3:** Allow a simple masked preview of stored project secrets in protected administration: a few initial and final characters, with the middle hidden. Full-value reveal is not required by this decision. The exact preview length is an implementation detail; short secrets must remain fully masked where necessary.

## Confirmed server pairing

For an HA deployment, configure trusted peer public keys reciprocally and provide a reachable peer endpoint for the connecting side. Only one member needs a publicly reachable inter-server endpoint; the other can initiate an outbound connection from behind NAT/firewall. Reciprocal trust does not require reciprocal inbound reachability. Each server retains its own private keys; no private key needs to be entered on the peer.

Display the server's public identity key in full for copying into peer configuration. It is public pairing material, unlike the masked preview of project secrets. Authenticate possession of the corresponding private key before admitting the configured peer; an address or unverified presented public key alone is not sufficient.

## Confirmed server identification requirement

Every server must be clearly identifiable by name/identity in logs and throughout system diagnostics and administration. Multiple servers may archive to the same S3 destination, so each archive must have an unambiguous originating server. The user has not selected hashing a key as the identifier scheme.

Recommend a human-readable configurable server name plus a generated persistent instance identifier. The name is for operators; the identifier distinguishes instances even if names are reused. Display both where useful, and retain the instance identifier across restarts, renaming, and key rotation. A public-key fingerprint is separate peer-verification metadata, not the only log identity.

The instance identifier is host/protocol metadata, not a branded application entity ID. Existing Flow entity IDs and references remain positive numeric values. Choose the exact identifier encoding during implementation; do not introduce textual type prefixes into application IDs.

Persist instance identity with first-start state. A new server gets its own identity; restoring the same server preserves it. Define an explicit clone/new-instance operation so copying a deployment cannot silently make two active servers claim one identity. Reject duplicate identities during peer configuration/admission. Use the stable instance identifier for S3 namespaces; readable names remain metadata, so renaming does not split or overwrite archive ownership.

## Proposed key and peer lifecycle

Use a stable server identity key pair for peer authentication and appropriately separated local encryption keys for stored secrets. The exact algorithms and key hierarchy are implementation choices to evaluate with reviewed libraries. Do not reuse one raw key for incompatible cryptographic purposes.

Initialize keys and the encrypted store safely, with concurrent-start protection and recoverable initialization state. If existing encrypted data is present but its key is missing or invalid, report a recovery error instead of silently generating a new key that cannot decrypt the data. Support explicit key rotation while preserving decryptability and configured peer trust.

Establish an encrypted peer channel with mutual proof against the configured public keys. Carry bidirectional communication over one established connection; either application role can initiate it when the network topology allows. Use bounded reconnect/backoff and resume synchronization from durable checkpoints. Do not require the listening side to open a new inbound connection to an unreachable dial-only member. Keep this on a protected inter-server surface, separate from public application routes and the existing protected admin UI. Validate peer configuration candidates before activation; do not trust discovery results automatically. Pairing config, connection status, and fingerprints belong in protected server administration.

Replicate project configuration and secrets over authenticated encrypted channels and protect secrets under the receiving server's own storage keys. Keep each member's private identity/encryption keys and local log state private to that member. Pairing is a host-level trust relationship; it does not implicitly grant cross-project application identities, permissions, or entity access.

## HA scope and open architecture

- **HA-1:** The deployment has one primary for the whole server and one or more standby members. A standby is prepared for full-server takeover; it is not an independently active writer for selected projects.
- **HA-4:** Only one member needs public inter-server reachability. The other may dial out, with both directions of peer traffic using the established authenticated channel. Network listener/dialer roles are independent of primary/standby application roles.

- **HA-2:** All HA members run the same software and host all the same projects. Each member holds a full replica of every project database. Replicate the deployment as a whole; do not shard projects/data, assign different projects to different members, or build a Kubernetes-like scheduler. Simplicity and availability are the goals, not distributed throughput.

Keep project programs, schemas, application configuration/secrets, business data, blobs, audit, ID sequences, and durable event/effect state consistent across members. Replication preserves the same project's identities/IDs across its replicas; it does not merge identities or entities between different projects. Cluster membership explicitly authorizes full-deployment replication, while ordinary application access still uses each project's permissions.

Server identity/private keys, peer endpoint settings, host resource settings, and operational logs remain member-local. Each member still archives only its own logs. Do not execute handlers or external effects merely while applying replicated committed state. Only the primary serves mutating application work and dispatches project jobs; promotion transfers that responsibility for the entire deployment. Read-serving and client ingress details still need implementation decisions.

- **HA-5:** Standby takeover is automatic. Promotion must establish exclusive primary ownership and verified committed state before accepting writes or dispatching effects; a missed heartbeat alone is not permission to bypass those checks. Returning former primaries must rejoin as standby after state reconciliation, not resume stale primary work.

Provide tested defaults for health checks, reconnect, promotion timing, and recovery limits. Do not require operators to design a failover protocol in the wizard. If safe ownership/durability cannot be established, surface the problem and preserve data rather than forcing competing writers.

- **HA-3:** Reliability and not losing data are the primary goals, ahead of throughput. Do not acknowledge successful application writes or accepted business-event input until the required primary/standby copies are durably committed under the replication protocol. Mere eventual full copying does not satisfy this requirement. When the required replica cannot confirm durability, stop acknowledging new writes and report the unavailable write state instead of silently continuing in a potentially lossy mode.

Preserve data, atomic audit, numeric allocation state, configuration, and accepted event/effect state together at the acknowledged replication boundary. After reconnect, reconcile committed/uncertain operations without duplicating mutations or effects. Use request/event deduplication for retries after a response is lost. Safe promotion must prevent both disconnected members from accepting writes as primary. Specify and test the supported failure/recovery model rather than claiming full copies alone protect against every possible simultaneous disaster. The initial implementation should use a small fixed-role primary/standby model, not per-project placement or a general workload orchestrator.

Define public application ingress separately from peer connectivity. An outbound peer connection solves synchronization reachability, but does not by itself keep applications publicly reachable after the only public member fails. Options include a stable public proxy/tunnel entrypoint or another explicitly reachable fallback. The ingress arrangement is an open deployment decision, not an implicit requirement that both HA members expose inbound ports.

Full database replication must preserve application mutations together with audit, committed numeric ID sequences, durable events, and effect completion state. Prevent concurrent ownership and duplicate external effects during disconnects/recovery. Program/schema migration and secret rotation must coordinate with the chosen replica model. Address/public-key exchange establishes trust, not data replication or leader election by itself.

## Backup recovery

The user requires configurable backups that can actually restore the deployment. Key initialization, encryption, and server identity must support recovery on the original or a replacement server. See [backup contents, key recovery, and restoration](backups.md); do not generate a fresh undecryptable key and declare encrypted data restored.

## Open decisions

- **CFG-2:** Which settings must apply live? Recommend live application settings, connector credentials, and logging settings; label settings requiring listener/process restart clearly.
- **HA-6:** How should public application traffic reach the standby after loss of the only public member? Define a stable proxy/tunnel or reachable fallback independently of the peer-link direction.

Remaining answers: Pending conversation. Unanswered recommendations are not approved decisions.

## Dependencies and verification

This is the first foundation for [routing/TLS](https.md), [external integrations](external-integrations.md), and [authentication](external%20auth.md). The protected admin listener remains the management surface; its HTTP console keeps ordinary application permissions.

Verify server name/identity persistence, rename and key rotation without identity changes, distinct instances sharing one archive bucket, clone/duplicate-identity handling, server-key generation/reuse, concurrent first startup, missing/corrupt-key recovery, rejected unknown or mismatched peers, mutual authentication, identical software/project inventory and full database copies, full-server primary/standby roles, no handlers/effects during replica application, a dial-only member behind NAT, both role/network-direction combinations, connection loss/resume, automatic takeover and safe old-primary rejoin, confirmed writes/events surviving primary loss, no success acknowledgement during required-replica disconnection, stale-owner rejection, ambiguous-response retry deduplication, public-ingress failover under the selected deployment contract, restart and rotation, generated-default reuse, concurrent initialization, encryption-at-rest, project isolation, invalid-candidate rollback, restart recovery, rotation, masked-preview behavior for short and long values, absence of full secrets from preview responses, redaction, and version conflicts. Test the settings flow against the real runtime with an isolated database. Keep application mutation audit atomic; record configuration administration with redacted metadata and never secret values.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
