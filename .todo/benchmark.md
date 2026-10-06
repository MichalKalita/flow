# Benchmark and capacity guidance

## Requested outcome

Assess CPU, RAM, storage performance, available disk space, and the workload's limiting resource. Guide administrators toward suitable request concurrency and log storage settings from the admin panel.

## Current implementation

Resource sampling and bounded latency histograms already exist. Tokio workers, blocking threads, and HTTP admission scale with CPU and host/cgroup memory, with explicit overrides. SQLite execution is serialized per project, so higher request concurrency does not automatically increase one project's database throughput. `FLOW_HTTP_ADMISSION=unlimited` exists; it is a configured mode, not a safe recommendation derived from an empty-host test.

The repository has k6 traffic and breakpoint scripts. See [the workload guide](../k6/README.md), [resource sampling](../runtime/src/resources.rs), [telemetry](../runtime/src/telemetry.rs), and [HTTP integration tests](../runtime/tests/http.rs).

## Implementation options and recommendation

1. Passive guidance from real traffic and resource measurements. Recommend this as the always-available baseline.
2. An opt-in bounded diagnostic run using isolated synthetic data and capped CPU, memory, temporary storage, duration, and concurrency. Recommend this for initial calibration.
3. Full workload saturation using the existing external k6 scripts. Keep this as an explicitly initiated capacity test, not an automatic background task on production.

No single microbenchmark can determine the best settings for every Flow program. Report the tested mix, data size, environment, bottleneck evidence, and uncertainty. Include read/write ratios, large payloads, audit overhead, event processing, and multi-project contention. A synthetic result is not a production capacity guarantee.

## Proposed admin workflow

Inventory usable CPUs, memory limit, filesystem free space, application/WAL/audit sizes, and server-wide operational-log usage. Include the configured local byte target, oldest retained timestamp/age, and pending S3 export bytes when export is enabled. Distinguish host facts from estimates and unavailable platform measurements.

Show current worker/admission overrides and the bounded default. Offer an isolated calibration profile with progress and cancellation. Measure admitted throughput, rejected requests, errors, latency histograms, CPU/RSS, and temporary-disk usage at a small set of concurrency levels. Stop at configured resource limits; leave production data untouched.

Produce recommendations with headroom, not a single universal “best” number. Let the administrator apply supported settings explicitly. Reserve space for databases, WAL, retained audit, accepted events, migrations, and backups before proposing the server-wide log budget. Treat that budget as an approximate long-term target with brief bounded overshoot, and report actual retained time coverage. Audit has no automatic retention today and must not be silently deleted to satisfy a benchmark recommendation.

## Open decisions

- **CAP-1:** May active calibration run while production traffic is present? Recommend passive guidance by default and opt-in active tests with strict limits or a maintenance window.
- **CAP-2:** What latency/error target defines acceptable capacity? Recommend a project-specific target; provide a clearly labeled example, not an assumed global SLO.
- **CAP-3:** Should guidance automatically change settings? Recommend suggestions plus explicit application, with version checks and a way to restore prior settings.

Answers: Pending conversation.

## Dependencies and verification

Uses [live settings](variables-and-secrets.md) and [the global log budget](runtime%20logs.md). Start with passive inventory early; repeat representative capacity validation as events, integrations, and mail are introduced.

Verify cancellation, isolated data, bounded allocation, cleanup after restart, disk pressure, CPU/memory detection, admission rejections, and recommendations on both the 1 CPU / 2 GB minimum and larger hosts. Store bounded aggregates rather than individual latency samples. The whole system must run without an external monitoring stack.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
