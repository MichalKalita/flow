# Benchmark and capacity guidance

## Product purpose

Flow aims to provide reliable, easy-to-operate software for companies on inexpensive servers. Reliability and preserving data take priority over throughput. It is not a Kubernetes-style orchestrator or a platform intended for thousands of nodes. The primary audience has no infrastructure expertise. Measure success by a working reliable deployment and understandable recovery workflows, not by the number of tuning controls exposed. Capacity advice should improve the full server's reliability and use of available hardware rather than recommend sharding, scheduling clusters, or required infrastructure stacks.

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

## Confirmed default and wizard requirement

Every setting has a good operational default and guided configuration. Users should not need performance expertise or a benchmark run just to obtain a reliable installation. The wizard recommends suitable values for the particular server, including worker/admission limits, server-wide log target and chunk size, storage headroom, and enabled HA/S3 capabilities. Recommendations complement defaults; the wizard is not mandatory for a valid standalone startup.

Use a shared settings schema and recommendation service rather than separate hard-coded values in startup and the UI. Show detected facts, current/default values, proposed changes, reasons, and uncertainty. Retain deliberate administrator overrides. Required external details such as peer keys and storage credentials are collected explicitly; recommendations cannot fabricate them.

Start with bounded passive inventory. Optional active diagnostics can refine estimates, but must not run saturation tests automatically merely because the wizard opened. A new installation still receives usable default guidance without prior traffic history. Calibrate defaults and numeric formulas during implementation and validate them against representative workloads and the minimum host. The user confirms good defaults as a product priority; low-level tuning values should normally be selected by the system, with optional expert overrides.

## Proposed admin workflow

Inventory usable CPUs, memory limit, filesystem free space, application/WAL/audit sizes, and server-wide operational-log usage. Include the independently configured local byte target and chunk-size/export threshold, oldest retained timestamp/age, and pending S3 export bytes when export is enabled. Account for bounded remote log-query caching, full replica storage, and backup/restore staging, schedule, and retention. Label measurements and recommendations with the readable server name and stable instance identity. Distinguish host facts from estimates and unavailable platform measurements.

Show current worker/admission overrides and the bounded default. Offer an isolated calibration profile with progress and cancellation. Measure admitted throughput, rejected requests, errors, latency histograms, replication durability/wait state when HA is enabled, CPU/RSS, and temporary-disk usage at a small set of concurrency levels. Stop at configured resource limits; leave production data untouched.

Produce recommendations with headroom, not a single universal “best” number. Let the administrator review and apply the recommended settings as a validated candidate. Reserve space for databases, WAL, retained audit, accepted events, migrations, and backups before proposing the server-wide log budget. Treat that budget as an approximate long-term target with brief bounded overshoot, and report actual retained time coverage. Audit has no automatic retention today and must not be silently deleted to satisfy a benchmark recommendation.

## Capacity of full-server HA replicas

HA is a primary/standby copy of the entire deployment: identical software, all projects, and complete databases on every member. Assess whether each member can run the full workload after promotion and has space for all databases, audit, blobs, event state, and replication recovery. Do not recommend sharding projects or distributed scheduling as the way to satisfy capacity. Include replication overhead in bounded diagnostics while retaining member-local log targets and S3 export identity.

## Open decisions

- **CAP-1:** May active calibration run while production traffic is present? Recommend passive guidance by default and opt-in active tests with strict limits or a maintenance window.
- **CAP-2 (implementation guidance):** Ship a tested baseline reliability/headroom profile; workload-specific latency/error targets may be optional expert settings, not required setup questions.
- **CAP-3 (implementation guidance):** Apply good host-derived defaults automatically on initialization. Show wizard recommendations before replacing existing explicit settings, with version checks and restoration of prior values.

Answers: Pending conversation.

## Dependencies and verification

Uses [live settings](variables-and-secrets.md) and [the global log budget](runtime%20logs.md). Start with passive inventory early; repeat representative capacity validation as events, integrations, and mail are introduced.

Verify cancellation, isolated data, bounded allocation, cleanup after restart, disk pressure, CPU/memory detection, admission rejections, and recommendations on both the 1 CPU / 2 GB minimum and larger hosts. Store bounded aggregates rather than individual latency samples. The whole system must run without an external monitoring stack.

## Implementation order

See [the shared implementation plan](implementation-plan.md) for delivery order, dependencies, milestones, and the decision queue.
