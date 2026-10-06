# Server-wide runtime logs

## Requested outcome and confirmed scope

All operational logs belong to the server as a whole. Provide one server-level logging service, archive set, storage budget, retention policy, and optional S3 archival configuration. Projects are metadata for filtering and verbosity rules, not separate log stores or retention owners. Every log carries its originating server instance identifier and readable server name.

The admin interface must provide search, severity labels, a histogram, and the timestamp/age of the oldest retained log. Logging settings are changed through protected server administration. Supply defaults for the local target, chunk size, cleanup cadence, verbosity, and archival policy; the shared configuration wizard recommends appropriate values based on storage headroom and observed log volume.

## Current implementation

The existing implementation uses separate project observers/directories plus a host observer. It has a 200-event memory cache and bounded 512-event writer queue per observer, rotating JSONL files at 256 MiB with three archives. Project archive queries scan at most 4 MiB and return at most 200 entries; combined system browsing uses recent buffers. The UI already filters severity, kind, endpoint, status, text, and time.

This is the starting point, not the requested final storage model. Consolidate operational logs into the common server service; keep project identity in each record. Project application databases, mutation audit, and telemetry isolation remain separate concerns. See [observability](../runtime/src/observability.rs), [archive queries](../runtime/src/log_store.rs), [the explorer](../admin-ui/src/logs.tsx), and [tests](../runtime/tests/observability.rs).

## Confirmed retention behavior

- **LOG-1:** Overwrite old history by removing the oldest chunks as new logs arrive. The configured byte limit is a long-term target, not a strictly atomic write boundary. Exceeding it briefly for a few seconds is acceptable; a 1 GB setting should converge to approximately 1 GB retained locally over time.
- **LOG-3:** Show the oldest locally retained log's timestamp and age in administration. The duration of retained history depends on traffic volume; report observed coverage rather than converting bytes into an assumed fixed number of days.
- Optional export of old logs to S3-compatible storage is configured for the entire server, not per project. Archives can contain records from multiple projects under protected administrative access.
- **LOG-6:** Configure the chunk size/export threshold independently of the total local log target. Once a chunk is complete and closed, schedule its upload immediately, well before local retention is exhausted. A 50 MB chunk is an illustrative setting, not a fixed required default.
- **LOG-7:** Each server, including every HA member, logs and exports only its own operational logs. Do not replicate log files or make another HA member their export owner.
- **LOG-8:** Every record and exported segment identifies its originating server unambiguously. Multiple servers can write to the same S3 destination without namespace collisions.
- **LOG-9:** The same log explorer filters and reads S3 history when logs are no longer local. In HA, every member returns its own local results and the request-receiving server gathers them, queries S3, and returns one combined answer.

## Proposed storage and cleanup

Use append-only segments with a server-wide writer and background rotation/cleanup. Expose the local retention byte target and chunk-size/export threshold as separate server settings. Validate that the chunk size is compatible with the local storage budget. Remove the oldest closed segments in chunks instead of deleting an individual line for every new line. Segment size and cleanup cadence should make overshoot small and temporary; prolonged inability to converge must be visible as a storage/retention problem.

Count all local operational log segments, including files awaiting export, toward the target. Do not create an unbounded second spool outside the measured log budget. Bound memory queues by entries and bytes. Cleanup must never touch application data, mutation audit, or accepted business-event payloads. Log writes and cleanup need not form an atomic transaction with each other or with application writes.

Maintain bounded segment metadata: byte count, first/last record timestamps, and lifecycle/export status. Derive the oldest retained record from actual surviving segments, handling empty files and cleanup/restart. For a filtered project view, optionally show that filter's coverage separately from the server-wide oldest log. A configured budget reduction should be followed by background cleanup toward the new target; it does not require stopping the server.

Plan transition from existing per-project archives without destroying available history unexpectedly. Options are one-time ingestion into server segments or a temporary read-only legacy view during migration. New writes must have one server owner. Public applications must not gain access to server archives through project APIs.

## Logs in HA deployments

Each HA member owns its own server log service, local byte target, chunk/export threshold, oldest-log age, and archival status. Each member uploads only its own chunks. Pairing servers does not merge or replicate operational logs or transfer their export ownership. Use stable server-instance remote object prefixes so archives from different members cannot collide. Store readable name and stable instance identity in archive metadata/manifests as well as records. Preserve the identity recorded when a log was emitted; do not rewrite old records to a new name on rename. Cross-server log queries use authenticated, authorized peer requests; this shares query results for administration without replicating member log stores.

## Optional server-level S3 archival

Configure endpoint, bucket, server-specific key prefix, secret references, and archival policy in protected server settings. Use the same S3 connector infrastructure as application storage, but with explicitly separate host credentials and namespace. No project decides the destination for other projects' logs.

Upload closed immutable segments through the common job/external-effect mechanism. Closing a chunk triggers immediate scheduling of its upload; do not wait until that chunk is about to leave local retention. With a 1 GB local target and 50 MB chunks, export starts as each roughly 50 MB chunk becomes available, while local history continues toward the overall target. Successful upload does not itself evict the local copy; ordinary local retention still determines its removal. Use stable object names and completion metadata to make retries/restart safe. Verify successful storage before labeling an archive exported. Optional compression and manifests should remain bounded and should not introduce a separate scheduler or event engine.

Show both the total local byte target and configured chunk-size threshold, plus export enabled/disabled, queued bytes, progress/last success, and failures alongside local retained bytes, target, oldest timestamp, and age. Show local oldest-log age separately from the oldest available S3 history, so administrators understand both retention tiers. Remote archival history is searchable through the same explorer when export/read access is configured. Remote retention still needs a policy.

The outcome when S3 is unavailable and local history needs eviction is still open. Retrying indefinitely while retaining all files would contradict the bounded server log target. Use the common configurable attempt/terminal policy, with clear storage-pressure behavior and visible gaps. Upload diagnostics must not create an uncontrolled feedback loop of more archives and more upload failures.

## Confirmed coordinated local and S3 log explorer

The server receiving the protected administrative query coordinates the response. Send the same filters to each configured HA member; each member returns only its own bounded local log results. The coordinator also reads applicable S3 history and merges everything into one response. In a standalone deployment, this is the same flow with one local member.

Use common filters for time, severity, kind, endpoint, status, text, project, and server identity. Query only configured deployment/member archive prefixes, not arbitrary buckets or keys supplied by the client. Upload credentials remain scoped to each server's own namespace; configure coordinator read authority explicitly for the participating members' archives.

Prefer local records where available and deduplicate local/S3 overlap by stable originating-server and record identity. Preserve original server names/identifiers, order deterministically by timestamp with an identity tie-breaker, and paginate the combined result. A cursor must capture progress across members and remote chunks without causing duplicate/missing pages merely because a chunk was uploaded or evicted locally.

Use bounded archive manifests with time coverage and server identity to select chunks. Stream remote reading/decompression with bounded bytes, memory, deadlines, cache size, and concurrency; do not download the full bucket for every filter operation or require an external search stack. A short recent query need not scan irrelevant historical S3 objects.

Merge histogram counts under the same filter/time window, deduplicating overlap. Return completeness/coverage metadata: unavailable peer, inaccessible/missing S3 chunk, exceeded scan budget, or timeout. Return available results with explicit partial status rather than labeling an incomplete query complete. Continuation can resume a bounded scan. Operational status and oldest-history coverage distinguish local from remote availability.

The request receiver is a temporary coordinator, not a permanent separate log server. Every member continues owning and exporting its own logs. Aggregation is a protected administration capability and does not grant ordinary applications access to mixed-project or peer archives.

## Server identity in operational views

Show server name and stable instance identity in administration, archive/export status, oldest-log coverage, and log details. Carry originating identity through common event/job diagnostics, resource/telemetry views, startup/error output, and request correlation metadata. Replicated business audit records retain the original mutation server rather than acquiring the reader/replica's identity; application entity IDs remain numeric and project-local.

Do not use display names as authentication or authorization. Peer trust remains pinned to configured public keys. See [server identification](variables-and-secrets.md#confirmed-server-identification-requirement).

## Search, severity, and histogram

Use server-level archived search with project as an optional filter. Preserve bounded scans, pagination, and cursor expiry when old segments are removed. Histogram buckets use the same filters and label partial archive scans as partial.

Define one severity vocabulary such as ERROR, WARN, INFO, DEBUG, and TRACE; map “verbose” to a documented setting. Server configuration can set the default and override verbosity for a named project. These are filtering rules in one host service, not independent project log budgets. No severity may reveal credentials, tokens, request bodies, email contents, signed URLs, or plugin arguments.

Operational log storage or export failure must not undo committed business writes or block requests indefinitely. Persist settings using the protected configuration store.

## Open decisions

- **LOG-2:** Which default histogram window should be shown? Recommend one hour, with bounded 15-minute and six-hour alternatives.
- **LOG-4:** If S3 is unavailable when old segments must be removed, should the server evict them to maintain local retention or keep them and apply backpressure/drop new operational logs? Recommend bounded retries followed by local eviction with an explicit export-gap warning; the chosen policy must not grow disk usage indefinitely.
- **LOG-5:** How long should remote archives remain in S3? Recommend an explicit server archival retention policy, independent of the local byte target, with clear ownership of remote deletion.

Remaining answers: Pending conversation. Server-wide ownership, approximate byte-target retention, oldest-log visibility, optional S3 archival, upload on chunk completion, per-server export ownership in HA, and combined local/S3 exploration are confirmed.

## Dependencies and verification

Local logging depends on [live settings](variables-and-secrets.md) and feeds [capacity guidance](benchmark.md). Optional remote archival follows [the common event/effect infrastructure](external-integrations.md) and a minimal [S3 connector](s3.md); it does not delay delivery of bounded local logs.

Verify server ownership across many projects, convergence after brief overshoot, rotation/deletion races, lowering the target, oldest-age accuracy, empty history, restart, legacy archives, filter/cursor behavior, bounded histogram scans, and redaction. Test multi-member query fan-out, request-receiver coordination, S3-only history with identical filters, local/remote deduplication, merged pagination/histograms, partial responses and bounded downloads, independent chunk/local-target settings, upload scheduling immediately after chunk closure, local retention after successful export, separate HA-member archives/export workers, server identity in records/manifests/UI, rename and key rotation preserving archive namespaces, S3 outages, duplicate/restarted uploads, bounded pending bytes, exported-state accuracy, and the selected pressure/remote-retention policy. Include admin E2E for local coverage and export status. Keep logs and telemetry outside application SQLite; audit retention is separate.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
