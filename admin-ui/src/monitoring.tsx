import { useState } from "preact/hooks";
import {
  Badge,
  Chart,
  Empty,
  Icon,
  Panel,
  ResourceBar,
  Stat,
} from "./components";
import {
  bytes,
  colors,
  duration,
  formatNumber,
  latency,
  rate,
  requestSeries,
  serviceSeries,
  windowStats,
} from "./data";
import type { Overview, Metric } from "./types";
type Props = {
  data: Overview;
  minutes: number;
  onConsole: (name: string) => void;
  onLogs: (endpoint?: string) => void;
};
const requestLines = [
  { key: "count", label: "Requests", color: colors.green },
  { key: "errors", label: "Errors", color: colors.red },
];
const latencyLines = [
  { key: "mean_ms", label: "Mean", color: colors.green },
  { key: "p95_ms", label: "p95", color: colors.orange },
  { key: "p99_ms", label: "p99", color: colors.purple },
];
export function OverviewPage({ data, minutes, onConsole, onLogs }: Props) {
  const metrics = data.metrics.endpoints,
    chartMetrics = data.metrics.system
      ? { system: data.metrics.system }
      : metrics,
    stats = windowStats(
      chartMetrics,
      minutes,
      data.metrics.histogram_bounds_ms,
    ),
    points = requestSeries(chartMetrics, minutes),
    service = data.metrics.service;
  const total = Object.values(metrics).reduce((n, m) => n + m.count, 0);
  const active =
    (service.gauges.mqtt_connections ?? 0) +
    (service.gauges.ws_connections ?? 0);
  const top = Object.entries(metrics)
    .sort((a, b) => b[1].count - a[1].count)
    .slice(0, 5);
  return (
    <>
      <div class="stats-grid">
        <Stat
          label="HTTP requests"
          value={formatNumber(stats.count)}
          detail={`${formatNumber(total)} since first start`}
          icon="traffic"
        />
        <Stat
          label="Request throughput"
          value={
            <>
              {formatNumber(rate(data, "http_requests"), 2)}
              <small> /s</small>
            </>
          }
          detail="Rolling 60-second average"
          icon="overview"
        />
        <Stat
          label="p95 response time"
          value={latency(stats.p95)}
          detail="Selected window · histogram estimate"
          icon="resources"
        />
        <Stat
          label="Error rate"
          value={
            <>
              {formatNumber(stats.errorRate, 2)}
              <small> %</small>
            </>
          }
          detail={`${formatNumber(stats.errors)} responses with HTTP 4xx / 5xx`}
          tone={stats.errors ? "text-rose-600" : ""}
          icon="audit"
        />
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel
          title="Request volume"
          description="Application requests and errors per minute"
        >
          <Chart
            data={points}
            series={requestLines}
            unit="req/min"
            title="HTTP request count over time"
          />
        </Panel>
        <Panel
          title="Response latency"
          description="Mean and tail latency across all endpoints"
        >
          <Chart
            data={points}
            series={latencyLines}
            unit="ms"
            title="HTTP latency percentiles over time"
          />
        </Panel>
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-[1.65fr_1fr] gap-5">
        <Panel
          title="Most active endpoints"
          description="Cumulative traffic, including failed requests"
          action={
            <button class="text-button" onClick={() => onLogs()}>
              Explore logs <Icon name="arrow" size={14} />
            </button>
          }
        >
          {top.length ? (
            <div class="table-wrap">
              <table>
                <thead>
                  <tr>
                    <th>Endpoint</th>
                    <th>Requests</th>
                    <th>p95</th>
                    <th>Errors</th>
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {top.map(([name, m]) => (
                    <tr key={name}>
                      <td>
                        <div class="endpoint-cell">
                          <Badge
                            tone={name.startsWith("GET") ? "green" : "orange"}
                          >
                            {name.split(" ")[0]}
                          </Badge>
                          <span class="mono">
                            {name.split(" ").slice(1).join(" ") || name}
                          </span>
                        </div>
                      </td>
                      <td>{formatNumber(m.count)}</td>
                      <td>{latency(m.p95_ms)}</td>
                      <td>
                        <span class={m.errors ? "text-rose-600" : "muted"}>
                          {formatNumber(m.errors)}
                        </span>
                      </td>
                      <td>
                        <button
                          class="icon-button"
                          title="Inspect request logs"
                          onClick={() => onLogs(name)}
                        >
                          <Icon name="logs" size={16} />
                        </button>
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          ) : (
            <Empty />
          )}
        </Panel>
        <Panel
          title="Runtime health"
          description="One process. Every part of your system."
        >
          <div class="health-list">
            <div>
              <span class="health-label">
                <i class="status-dot" />
                Process uptime
              </span>
              <strong>{duration(service.uptime_seconds)}</strong>
            </div>
            <div>
              <span>Resident memory</span>
              <strong>{bytes(data.resources.process.rss_bytes)}</strong>
            </div>
            <div>
              <span>Active MQTT / WebSocket</span>
              <strong>
                {active}
                <small> connections</small>
              </strong>
            </div>
            <div>
              <span>Committed stream events</span>
              <strong>
                {formatNumber(service.counters.stream_events ?? 0)}
              </strong>
            </div>
            <div>
              <span>Log writer queue</span>
              <strong>
                {data.resources.observability.queued_log_events ?? 0}
                <small>
                  {" / "}
                  {data.resources.observability.log_queue_limit ?? 512}
                </small>
              </strong>
            </div>
            <div>
              <span>Dropped logs / storage errors</span>
              <strong
                class={
                  data.metrics.dropped_logs || data.metrics.storage_errors
                    ? "text-rose-600"
                    : ""
                }
              >
                {formatNumber(data.metrics.dropped_logs)} /{" "}
                {formatNumber(data.metrics.storage_errors)}
              </strong>
            </div>
          </div>
          <div class="panel-note">
            Control plane and telemetry run inside this service. No external
            agents or monitoring database.
          </div>
        </Panel>
      </div>
      <Panel
        title="Application topology"
        description="Declared capabilities, served by a single runtime"
      >
        <div class="topology-grid">
          {[
            [
              "console",
              "HTTP operations",
              data.endpoints.filter((e) => e.method !== "WS").length,
              "Typed endpoints & central permissions",
            ],
            [
              "stream",
              "Streams",
              data.streams.length,
              "MQTT topics & retained history",
            ],
            [
              "traffic",
              "WebSocket queries",
              data.endpoints.filter((e) => e.method === "WS").length,
              "Live, permission-aware subscriptions",
            ],
            [
              "resources",
              "Automations",
              data.automations.length,
              "Transactional event handlers",
            ],
          ].map(([icon, label, value, text]) => (
            <div key={String(label)} class="topology-item">
              <Icon name={String(icon)} size={22} />
              <strong>{value}</strong>
              <span>{label}</span>
              <small>{text}</small>
            </div>
          ))}
        </div>
      </Panel>
    </>
  );
}
export function TrafficPage({ data, minutes, onConsole, onLogs }: Props) {
  const [filter, setFilter] = useState("all"),
    [search, setSearch] = useState("");
  const endpoints = data.endpoints.filter((e) => e.method !== "WS"),
    key = (e: (typeof endpoints)[number]) => `${e.method} ${e.path}`;
  const metrics: Record<string, Metric> =
    filter === "all"
      ? data.metrics.endpoints
      : data.metrics.endpoints[filter]
        ? { [filter]: data.metrics.endpoints[filter] }
        : {};
  const chartMetrics =
    data.metrics.system && filter === "all"
      ? { system: data.metrics.system }
      : metrics;
  const stats = windowStats(
      chartMetrics,
      minutes,
      data.metrics.histogram_bounds_ms,
    ),
    points = requestSeries(chartMetrics, minutes);
  return (
    <>
      <div class="page-toolbar">
        <div class="flex gap-3 items-center">
          <label class="muted">Endpoint</label>
          <select
            value={filter}
            disabled={!!data.metrics.system}
            onChange={(e) => setFilter(e.currentTarget.value)}
          >
            <option value="all">All endpoints</option>
            {endpoints.map((e) => (
              <option key={e.name} value={key(e)}>
                {key(e)}
              </option>
            ))}
          </select>
        </div>
        <Badge tone="blue">Minute aggregates · {minutes}m window</Badge>
      </div>
      {data.metrics.system && (
        <p class="muted">
          System charts combine all projects. Select a project in the sidebar
          for endpoint-specific history.
        </p>
      )}
      <div class="stats-grid">
        <Stat
          label="Requests in window"
          value={formatNumber(stats.count)}
          detail="Includes successful and failed calls"
          icon="traffic"
        />
        <Stat
          label="Mean response"
          value={latency(stats.mean)}
          detail="Weighted by request count"
          icon="resources"
        />
        <Stat
          label="p95 / p99"
          value={latency(stats.p95)}
          detail={`p99 ${latency(stats.p99)}`}
          icon="overview"
        />
        <Stat
          label="HTTP errors"
          value={formatNumber(stats.errors)}
          detail={`${formatNumber(stats.errorRate, 2)}% of requests`}
          icon="audit"
        />
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel title="Requests & errors" description="Count per minute">
          <Chart
            data={points}
            series={requestLines}
            unit="req/min"
            title="Endpoint request volume"
          />
        </Panel>
        <Panel
          title="Latency distribution over time"
          description="Upper histogram boundaries for p95 and p99"
        >
          <Chart
            data={points}
            series={latencyLines}
            unit="ms"
            title="Endpoint response latency"
          />
        </Panel>
      </div>
      <Panel
        title="Endpoint inventory"
        description="Live traffic statistics and declared operations"
        action={
          <div class="search-field">
            <Icon name="search" size={15} />
            <input
              placeholder="Find an endpoint…"
              value={search}
              onInput={(e) => setSearch(e.currentTarget.value)}
              aria-label="Search endpoints"
            />
          </div>
        }
      >
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Endpoint / operation</th>
                <th>Requests</th>
                <th>Mean</th>
                <th>p95</th>
                <th>Errors</th>
                <th>Actions</th>
              </tr>
            </thead>
            <tbody>
              {endpoints
                .filter((e) =>
                  `${e.path} ${e.name}`
                    .toLowerCase()
                    .includes(search.toLowerCase()),
                )
                .map((e) => {
                  const m = data.metrics.endpoints[key(e)];
                  return (
                    <tr key={e.name}>
                      <td>
                        <div class="endpoint-cell">
                          <Badge tone={e.method === "GET" ? "green" : "orange"}>
                            {e.method}
                          </Badge>
                          <span class="mono">{e.path}</span>
                        </div>
                        <small class="table-subtitle">
                          {e.name} {e.mutation ? "· mutation" : "· query"}
                        </small>
                      </td>
                      <td>{formatNumber(m?.count ?? 0)}</td>
                      <td>{latency(m?.mean_ms)}</td>
                      <td>{latency(m?.p95_ms)}</td>
                      <td class={m?.errors ? "text-rose-600" : "muted"}>
                        {formatNumber(m?.errors ?? 0)}
                      </td>
                      <td>
                        <div class="flex gap-2">
                          <button
                            class="icon-button"
                            title="Open in HTTP console"
                            onClick={() => onConsole(e.name)}
                          >
                            <Icon name="console" size={16} />
                          </button>
                          <button
                            class="icon-button"
                            title="Inspect logs"
                            onClick={() => onLogs(key(e))}
                          >
                            <Icon name="logs" size={16} />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
            </tbody>
          </table>
        </div>
      </Panel>
    </>
  );
}
export function StreamingPage({ data, minutes }: Props) {
  const s = data.metrics.service,
    g = s.gauges,
    c = s.counters;
  const connections = serviceSeries(s.history, minutes, [
    { key: "mqtt_connections", category: "gauges" },
    { key: "ws_connections", category: "gauges" },
  ]);
  const events = serviceSeries(s.history, minutes, [
    { key: "stream_events", category: "counts" },
    { key: "automation_runs", category: "counts" },
  ]);
  const deliveries = serviceSeries(s.history, minutes, [
    { key: "mqtt_delivered", category: "counts" },
    { key: "ws_delivered", category: "counts" },
  ]);
  return (
    <>
      <div class="stats-grid">
        <Stat
          label="MQTT connections"
          value={g.mqtt_connections ?? 0}
          detail={`${g.mqtt_sessions ?? 0} authenticated · ${g.mqtt_subscriptions ?? 0} subscriptions`}
          icon="stream"
        />
        <Stat
          label="WebSocket connections"
          value={g.ws_connections ?? 0}
          detail={`${g.ws_subscriptions ?? 0} active query subscriptions`}
          icon="traffic"
        />
        <Stat
          label="Stream throughput"
          value={
            <>
              {formatNumber(rate(data, "stream_events"), 2)}
              <small> /s</small>
            </>
          }
          detail="Committed events · rolling 60 seconds"
          icon="overview"
        />
        <Stat
          label="Messages delivered"
          value={formatNumber((c.mqtt_delivered ?? 0) + (c.ws_delivered ?? 0))}
          detail={`${formatNumber(c.mqtt_delivered ?? 0)} MQTT · ${formatNumber(c.ws_delivered ?? 0)} WS`}
          icon="arrow"
        />
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel
          title="Open connections"
          description="Minute-end samples of active transport connections"
        >
          <Chart
            data={connections}
            series={[
              { key: "mqtt_connections", label: "MQTT", color: colors.green },
              { key: "ws_connections", label: "WebSocket", color: colors.blue },
            ]}
            title="Active streaming connections"
            unit="connections"
          />
        </Panel>
        <Panel
          title="Event processing"
          description="Confirmed stream writes and executed automations"
        >
          <Chart
            data={events}
            series={[
              {
                key: "stream_events",
                label: "Stream events",
                color: colors.green,
              },
              {
                key: "automation_runs",
                label: "Automations",
                color: colors.orange,
              },
            ]}
            title="Events handled per minute"
            unit="events/min"
          />
        </Panel>
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-[1.5fr_1fr] gap-5">
        <Panel
          title="Outbound delivery"
          description="Successfully sent data messages, per minute"
        >
          <Chart
            data={deliveries}
            series={[
              { key: "mqtt_delivered", label: "MQTT", color: colors.green },
              { key: "ws_delivered", label: "WebSocket", color: colors.blue },
            ]}
            title="Message deliveries per minute"
            unit="messages/min"
          />
        </Panel>
        <Panel
          title="Transport counters"
          description="Cumulative since first telemetry snapshot"
        >
          <div class="health-list">
            {[
              ["MQTT publishes accepted", c.mqtt_received],
              ["MQTT delivery errors", c.mqtt_errors],
              ["WebSocket frames received", c.ws_received],
              ["WebSocket query errors", c.ws_errors],
              ["Automation executions", c.automation_runs],
              ["Stream events committed", c.stream_events],
            ].map(([label, value]) => (
              <div key={String(label)}>
                <span>{label}</span>
                <strong>{formatNumber(Number(value ?? 0))}</strong>
              </div>
            ))}
          </div>
        </Panel>
      </div>
      <Panel
        title="Stream catalogue"
        description="Retention and topic routing from the Flow program"
      >
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Stream</th>
                <th>MQTT topic</th>
                <th>Retention</th>
                <th>Message limit</th>
                <th>Automations</th>
              </tr>
            </thead>
            <tbody>
              {data.streams.map((s) => (
                <tr key={s.name}>
                  <td>
                    <strong>{s.name}</strong>
                  </td>
                  <td class="mono">{s.topic}</td>
                  <td>{duration(s.retention_seconds)}</td>
                  <td>{formatNumber(s.max_messages)}</td>
                  <td>
                    {data.automations
                      .filter((a) => a.source === s.name)
                      .map((a) => a.name)
                      .join(", ") || "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!data.streams.length && (
          <Empty
            title="No streams declared"
            text="MQTT topics and automations come from your Flow application."
          />
        )}
      </Panel>
    </>
  );
}
export function ResourcesPage({ data, minutes }: Props) {
  const r = data.resources,
    s = data.metrics.service,
    db = r.sqlite,
    o = r.observability;
  const memory = serviceSeries(s.history, minutes, [
      { key: "rss_bytes", category: "resources" },
    ]),
    cpu = serviceSeries(s.history, minutes, [
      { key: "cpu_percent", category: "resources" },
    ]);
  const componentRows = [
    ["SQLite page cache", db.page_cache_bytes, "SQLite counter"],
    ["SQLite schema", db.schema_bytes, "SQLite counter"],
    ["Prepared statements", db.prepared_statements_bytes, "SQLite counter"],
    ["Metrics & history", o.estimated_metrics_bytes, "Estimated heap capacity"],
    [
      "Dashboard log cache",
      o.estimated_log_buffer_bytes,
      "Estimated heap capacity",
    ],
    [
      "Log writer queue",
      o.estimated_log_queue_bytes,
      "Estimated heap capacity",
    ],
    ["Log write buffer", o.log_writer_buffer_bytes, "Fixed allocation"],
  ];
  const work = Object.entries(data.metrics.work ?? {}).sort(
    (a, b) => b[1].sum_ms - a[1].sum_ms,
  );
  return (
    <>
      <div class="stats-grid">
        <Stat
          label="Resident memory"
          value={bytes(r.process.rss_bytes)}
          detail={
            r.process.peak_rss_bytes
              ? `Peak ${bytes(r.process.peak_rss_bytes)}`
              : "Measured process RSS"
          }
          icon="resources"
        />
        <Stat
          label="Process CPU"
          value={
            <>
              {formatNumber(s.resources.cpu_percent, 1)}
              <small> %</small>
            </>
          }
          detail="100% = one CPU core · 5-second samples"
          icon="overview"
        />
        <Stat
          label="SQLite on disk"
          value={bytes(db.total_disk_bytes)}
          detail="Database + WAL + shared memory"
          icon="audit"
        />
        <Stat
          label="Telemetry storage"
          value={bytes(o.disk_bytes)}
          detail="Rotating logs + metric snapshot"
          icon="logs"
        />
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel
          title="Process memory"
          description="Minute-end resident memory samples"
        >
          <Chart
            data={memory}
            series={[{ key: "rss_bytes", label: "RSS", color: colors.green }]}
            formatter={(n) => bytes(n)}
            title="Process resident memory over time"
          />
        </Panel>
        <Panel
          title="CPU utilization"
          description="Process CPU time delta; not host-wide CPU usage"
        >
          <Chart
            data={cpu}
            series={[
              {
                key: "cpu_percent",
                label: "Process CPU",
                color: colors.orange,
              },
            ]}
            unit="%"
            title="Process CPU percentage over time"
          />
        </Panel>
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel
          title="Memory composition"
          description="Measured SQLite allocations and bounded-buffer estimates"
        >
          <div class="resource-budget">
            <ResourceBar
              label="Process RSS / host RAM"
              value={Number(r.process.rss_bytes ?? 0)}
              total={Number(r.host?.limit_bytes ?? 0)}
              detail={`${bytes(r.process.rss_bytes)} / ${bytes(r.host?.limit_bytes)}${
                r.host?.available_bytes != null
                  ? ` · ${bytes(r.host.available_bytes)} available`
                  : ""
              }`}
            />
          </div>
          <div class="table-wrap">
            <table>
              <thead>
                <tr>
                  <th>Component</th>
                  <th>Memory</th>
                  <th>Source</th>
                </tr>
              </thead>
              <tbody>
                {componentRows.map(([name, value, source]) => (
                  <tr key={String(name)}>
                    <td>{name}</td>
                    <td class="tabular">{bytes(value)}</td>
                    <td class="muted text-xs">{source}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          <div class="panel-note">
            RAM limit is physical memory, or the cgroup memory cap when the
            process is constrained. Estimates exclude allocator overhead, shared
            allocations, active requests and thread stacks. Components do not
            add up to RSS.
          </div>
        </Panel>
        <Panel
          title="Storage & backpressure"
          description="Disk log retention, SQLite files, and in-memory backpressure"
        >
          <div class="resource-budget">
            <ResourceBar
              label="Disk logs"
              value={Number(o.log_disk_bytes ?? 0)}
              total={Number(o.log_disk_limit_bytes ?? 0)}
              color={colors.blue}
              detail={`${bytes(o.log_disk_bytes)} / ${bytes(o.log_disk_limit_bytes)} rotating JSONL`}
            />
          </div>
          <div class="health-list">
            {[
              ["SQLite database", bytes(db.main_bytes)],
              ["Write-ahead log (WAL)", bytes(db.wal_bytes)],
              ["Shared-memory file", bytes(db.shm_bytes)],
              ["Logical SQLite pages", bytes(db.logical_bytes)],
              [
                "Dashboard log cache",
                `${o.log_buffer_limit ?? 200} recent events in RAM`,
              ],
              [
                "Log writer queue",
                `${o.queued_log_events ?? 0} / ${o.log_queue_limit ?? 512} waiting to flush`,
              ],
              ["Metric history", "6 hours · minute buckets"],
              ["HTTP in flight", `${s.gauges.http_inflight ?? 0} / 16`],
            ].map(([label, value]) => (
              <div key={String(label)}>
                <span>{label}</span>
                <strong>{value}</strong>
              </div>
            ))}
          </div>
          <div class="panel-note">
            Application logs rotate on disk up to the shown limit. The dashboard
            cache keeps the newest events in RAM for the live explorer. The
            writer queue is in-memory backpressure before those lines hit disk;
            a full queue drops operational log entries rather than blocking
            transactions. Audit data remains in SQLite.
          </div>
        </Panel>
      </div>
      <Panel
        title="I/O & plugin performance"
        description="Nested spans; cumulative timings include overlapping operations"
      >
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Operation</th>
                <th>Calls</th>
                <th>Mean</th>
                <th>p95</th>
                <th>Total time</th>
                <th>Failures</th>
              </tr>
            </thead>
            <tbody>
              {work.map(([name, m]) => (
                <tr key={name}>
                  <td class="mono">{name}</td>
                  <td>{formatNumber(m.count)}</td>
                  <td>{latency(m.mean_ms)}</td>
                  <td>{latency(m.p95_ms)}</td>
                  <td>{duration(m.sum_ms / 1000)}</td>
                  <td class={m.errors ? "text-rose-600" : "muted"}>
                    {m.errors}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!work.length && <Empty title="No I/O measurements yet" />}
      </Panel>
    </>
  );
}
