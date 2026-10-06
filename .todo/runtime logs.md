# Runtime logs

## Requested outcome

Configure a server-wide log storage limit, rather than only independent project limits. The admin interface must provide search, severity labels, and a log histogram. Project verbosity and logging settings must be editable in administration.

## Current implementation

The runtime has a 200-event memory cache, a bounded 512-event writer queue, rotating JSONL files, and archived search. Rotation is fixed at 256 MiB with three archives per observer. Per-project archive search is bounded to 4 MiB scanned and 200 returned entries per query; system browsing currently uses recent buffers. The UI already filters severity, kind, endpoint, status, text, and time. A host-wide configurable retention budget and a log-count histogram still need implementation.

See [observability](../runtime/src/observability.rs), [archive queries](../runtime/src/log_store.rs), [the log explorer](../admin-ui/src/logs.tsx), and [integration tests](../runtime/tests/observability.rs).

## Implementation options and recommendation

1. Split one host budget into fixed project allocations. Simple, but inactive projects reserve capacity that active projects could use.
2. Keep separate project files with a shared retention coordinator. Recommend this: preserve isolation while enforcing one global budget, with optional project caps and a reserved allowance for host errors.
3. Merge logs into a shared file/store. This complicates isolation and access filtering and is unnecessary for the requested result.

Count the active files and archives across all observers. Bound queues in both entries and bytes, since entry count alone does not bound memory. At budget pressure, prune eligible operational archives according to an explicit policy; never prune application databases, audit, or accepted event payloads through log cleanup. Show dropped entries, storage failures, and budget pressure.

## Search, severity, and histogram

Define one severity vocabulary such as ERROR, WARN, INFO, DEBUG, and TRACE; map “verbose” to a documented setting. Projects inherit a host default and can override verbosity within host resource ceilings. Greater verbosity must never expose credentials, tokens, request bodies, email contents, or plugin arguments.

Use bounded time buckets for log counts by severity. Recommend a live bounded aggregate for the recent window and resumable archive scans for older filtered ranges. A partial scan must be labeled partial; it cannot claim an exact histogram for an unscanned archive. Histograms describe retained/emitted log counts, not request latency percentiles.

Persist settings through the protected configuration store. Validate a lower budget and show the retention effect before applying it. Runtime logging failures must not undo committed application writes or block requests indefinitely.

## Open decisions

- **LOG-1:** How should global pressure select archives for removal? Recommend oldest eligible archives first, with a small protected host-error allowance and optional project caps.
- **LOG-2:** Which default time range should the histogram show? Recommend one hour, with bounded 15-minute and six-hour alternatives.
- **LOG-3:** Should lowering the budget prune archives immediately? Recommend an explicit preview followed by pruning under the newly applied policy.

Answers: Pending conversation.

## Dependencies and verification

Depends on [live settings](variables-and-secrets.md). Feeds [capacity guidance](benchmark.md) and every later feature's operational visibility.

Verify the total budget across many projects, rotation, cursor expiry, project deletion/reload, concurrent setting changes, disk-full handling, bounded histogram scans, and redaction at every severity. Add admin E2E coverage for settings and filtered histogram behavior. Keep logs and telemetry outside application SQLite; audit retention is a separate concern.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
