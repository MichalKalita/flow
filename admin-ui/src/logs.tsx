import { useEffect, useState } from "preact/hooks";
import { Badge, Drawer, Empty, ErrorBanner, Icon, Panel } from "./components";
import { formatNumber, latency } from "./data";
import { useDebounce, usePolling } from "./hooks";
import type { Api, Audit, Log, LogPage, Overview } from "./types";
export function LogsPage({
  api,
  data,
  initialEndpoint = "",
  interval,
  onCopy,
}: {
  api: Api;
  data: Overview;
  initialEndpoint?: string;
  interval: number;
  onCopy: (text: string) => void;
}) {
  const [search, setSearch] = useState(""),
    [kind, setKind] = useState(""),
    [level, setLevel] = useState(""),
    [endpoint, setEndpoint] = useState(initialEndpoint),
    [status, setStatus] = useState(""),
    [live, setLive] = useState(true),
    [cursors, setCursors] = useState<string[]>([]),
    [selected, setSelected] = useState<Log | null>(null),
    [range, setRange] = useState("0");
  const debounced = useDebounce(search);
  useEffect(() => {
    setCursors([]);
  }, [debounced, kind, level, endpoint, status, range]);
  const query = new URLSearchParams({
    search: debounced,
    kind,
    level,
    endpoint,
    status,
    limit: "100",
  });
  if (cursors.length) query.set("before", cursors.at(-1)!);
  if (range !== "0")
    query.set(
      "since",
      String(Math.floor(Date.now() / 60000) * 60 - Number(range) * 60),
    );
  const page = usePolling<LogPage>(
    api,
    `/api/logs?${query}`,
    live && !cursors.length ? interval : 0,
  );
  const exportPage = () => {
    const url = URL.createObjectURL(
      new Blob([JSON.stringify(page.data?.entries ?? [], null, 2)], {
        type: "application/json",
      }),
    );
    const link = document.createElement("a");
    link.href = url;
    link.download = "flow-logs.json";
    link.click();
    URL.revokeObjectURL(url);
  };
  return (
    <>
      <div class="page-toolbar">
        <div>
          <Badge tone={live && !cursors.length ? "green" : "neutral"}>
            {live && !cursors.length ? "Live tail" : "Browsing history"}
          </Badge>
          <span class="muted ml-3 text-sm">
            Memory buffer + rotating disk archives
          </span>
        </div>
        <div class="flex gap-2">
          <button class="button-secondary" onClick={exportPage}>
            <Icon name="logs" size={15} />
            Export page
          </button>
          <button
            class="button-secondary"
            onClick={() => {
              setLive(!live);
              setCursors([]);
            }}
          >
            <Icon name={live ? "pause" : "play"} size={15} />
            {live ? "Pause" : "Resume live"}
          </button>
        </div>
      </div>
      <Panel
        title="Log explorer"
        description="Search request IDs, endpoints, I/O operations and plugin calls"
      >
        <div class="filter-bar">
          <div class="search-field grow">
            <Icon name="search" size={16} />
            <input
              placeholder="Search logs, request IDs, operation names…"
              value={search}
              onInput={(e) => setSearch(e.currentTarget.value)}
              aria-label="Search log text"
            />
          </div>
          <select
            value={range}
            onChange={(e) => setRange(e.currentTarget.value)}
            aria-label="Log time range"
          >
            <option value="0">All retained logs</option>
            <option value="15">Last 15 minutes</option>
            <option value="60">Last hour</option>
            <option value="360">Last 6 hours</option>
          </select>
          <button
            class="icon-button"
            onClick={page.refresh}
            aria-label="Refresh logs"
          >
            <Icon name="refresh" />
          </button>
        </div>
        <div class="filter-bar secondary-filters">
          <select
            value={kind}
            onChange={(e) => setKind(e.currentTarget.value)}
            aria-label="Log kind"
          >
            <option value="">All kinds</option>
            {["request", "io", "plugin", "transport", "admin"].map((k) => (
              <option key={k}>{k}</option>
            ))}
          </select>
          <select
            value={level}
            onChange={(e) => setLevel(e.currentTarget.value)}
            aria-label="Log level"
          >
            <option value="">All levels</option>
            {["info", "warn", "error"].map((k) => (
              <option key={k}>{k}</option>
            ))}
          </select>
          <select
            value={endpoint}
            onChange={(e) => setEndpoint(e.currentTarget.value)}
            aria-label="Filter logs by endpoint"
          >
            <option value="">All endpoints</option>
            {data.endpoints.map((e) => (
              <option value={`${e.method} ${e.path}`} key={e.name}>
                {e.method} {e.path}
              </option>
            ))}
          </select>
          <select
            value={status}
            onChange={(e) => setStatus(e.currentTarget.value)}
            aria-label="HTTP status filter"
          >
            <option value="">All HTTP statuses</option>
            <option value="2xx">2xx · Success</option>
            <option value="4xx">4xx · Client error</option>
            <option value="5xx">5xx · Server error</option>
            <option value="errors">All errors</option>
          </select>
          <span class="muted ml-auto text-xs">
            {page.loading
              ? "Loading…"
              : `${page.data?.entries.length ?? 0} entries`}
          </span>
        </div>
        <ErrorBanner message={page.error} />
        <div class="table-wrap">
          <table class="log-table">
            <thead>
              <tr>
                <th>Time</th>
                <th>Level</th>
                <th>Kind</th>
                <th>Message / endpoint</th>
                <th>Status</th>
                <th>Duration</th>
              </tr>
            </thead>
            <tbody>
              {page.data?.entries.map((log) => (
                <tr
                  key={log.sequence}
                  class="clickable"
                  tabIndex={0}
                  onClick={() => setSelected(log)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") setSelected(log);
                  }}
                >
                  <td class="mono text-xs whitespace-nowrap">
                    <span>
                      {new Date(log.time).toLocaleTimeString("en-GB", {
                        hour12: false,
                      })}
                    </span>
                    <small class="table-subtitle">
                      {new Date(log.time).toLocaleDateString("en-GB")}
                    </small>
                  </td>
                  <td>
                    <Badge
                      tone={
                        log.level === "error"
                          ? "red"
                          : log.level === "warn"
                            ? "orange"
                            : "neutral"
                      }
                    >
                      {log.level}
                    </Badge>
                  </td>
                  <td class="muted">{log.kind}</td>
                  <td>
                    <span class="mono">
                      {log.endpoint ?? log.name ?? log.error ?? log.kind}
                    </span>
                    {log.request_id && (
                      <small class="table-subtitle mono">
                        {log.request_id}
                      </small>
                    )}
                  </td>
                  <td>
                    {log.status ? (
                      <Badge tone={log.status >= 400 ? "red" : "green"}>
                        {log.status}
                      </Badge>
                    ) : (
                      "—"
                    )}
                  </td>
                  <td class="whitespace-nowrap tabular">
                    {log.duration_ms != null ? latency(log.duration_ms) : "—"}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!page.loading && !page.data?.entries.length && (
          <Empty
            title="No matching logs"
            text="Adjust the filters or browse older archive pages."
          />
        )}
        {page.data?.cursor_expired && (
          <div class="panel-note">
            This archive has expired. Select Previous or Refresh logs to return
            to retained entries.
          </div>
        )}
        {page.data?.scan_limited && (
          <div class="panel-note">
            The bounded archive scan reached 4 MiB. Continue to scan older
            entries.
          </div>
        )}
        <div class="table-footer">
          <span>Page {cursors.length + 1} · newest first</span>
          <div class="flex gap-2">
            <button
              class="button-secondary"
              disabled={!cursors.length || page.loading}
              onClick={() => setCursors((c) => c.slice(0, -1))}
            >
              Previous
            </button>
            <button
              class="button-secondary"
              disabled={!page.data?.next_cursor || page.loading}
              onClick={() => {
                setLive(false);
                setCursors((c) => [...c, String(page.data!.next_cursor)]);
              }}
            >
              Older entries <Icon name="arrow" size={14} />
            </button>
          </div>
        </div>
      </Panel>
      {selected && (
        <Drawer title="Log details" onClose={() => setSelected(null)}>
          <div class="drawer-meta">
            <Badge tone={selected.level === "error" ? "red" : "blue"}>
              {selected.level}
            </Badge>
            <span>{selected.kind}</span>
            <small>{new Date(selected.time).toLocaleString()}</small>
          </div>
          {selected.request_id && (
            <button
              class="button-secondary"
              onClick={() => onCopy(selected.request_id!)}
            >
              <Icon name="copy" size={14} />
              Copy request ID
            </button>
          )}
          <pre class="code-block">{JSON.stringify(selected, null, 2)}</pre>
        </Drawer>
      )}
    </>
  );
}
export function AuditPage({
  api,
  data,
  onCopy,
}: {
  api: Api;
  data: Overview;
  onCopy: (text: string) => void;
}) {
  const [cursors, setCursors] = useState<number[]>([]),
    [entity, setEntity] = useState(""),
    [action, setAction] = useState(""),
    [transport, setTransport] = useState(""),
    [selected, setSelected] = useState<Audit | null>(null);
  const entities = [...new Set(data.streams.map((s) => s.name))];
  const query = new URLSearchParams({
    before: String(cursors.at(-1) ?? 0),
    entity,
    action,
    transport,
  });
  const page = usePolling<Audit[]>(api, `/api/audit?${query}`);
  useEffect(() => setCursors([]), [entity, action, transport]);
  const actor = (v: unknown) => {
    const a = v as { type?: string; id?: string };
    return a?.id ? `${a.type} / ${a.id}` : (a?.type ?? "—");
  };
  return (
    <>
      <div class="info-callout">
        <Icon name="audit" />
        <div>
          <strong>Every committed mutation has a record.</strong>
          <span>
            {" "}
            Audit entries commit atomically with application data, including
            stream retention and automations.
          </span>
        </div>
      </div>
      <Panel
        title="Mutation audit"
        description="Persistent, transactional history from SQLite"
      >
        <div class="filter-bar">
          <input
            placeholder="Entity name…"
            value={entity}
            onInput={(e) => setEntity(e.currentTarget.value)}
            aria-label="Filter audit entity"
            list="audit-entities"
          />
          <datalist id="audit-entities">
            {entities.map((e) => (
              <option key={e}>{e}</option>
            ))}
          </datalist>
          <select
            value={action}
            onChange={(e) => setAction(e.currentTarget.value)}
            aria-label="Audit action"
          >
            <option value="">All actions</option>
            {["INSERT", "UPDATE", "DELETE"].map((a) => (
              <option key={a}>{a}</option>
            ))}
          </select>
          <select
            value={transport}
            onChange={(e) => setTransport(e.currentTarget.value)}
            aria-label="Audit transport"
          >
            <option value="">All transports</option>
            {["HTTP", "MQTT", "event", "startup"].map((t) => (
              <option key={t}>{t}</option>
            ))}
          </select>
          <button
            class="button-secondary ml-auto"
            onClick={() => {
              setCursors([]);
              page.refresh();
            }}
          >
            <Icon name="refresh" size={14} />
            Latest
          </button>
        </div>
        <ErrorBanner message={page.error} />
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>ID / timestamp</th>
                <th>Action</th>
                <th>Entity</th>
                <th>Actor</th>
                <th>Operation</th>
                <th>Transport</th>
              </tr>
            </thead>
            <tbody>
              {page.data?.map((row) => (
                <tr
                  key={row.id}
                  class="clickable"
                  onClick={() => setSelected(row)}
                  tabIndex={0}
                  onKeyDown={(e) => {
                    if (e.key === "Enter") setSelected(row);
                  }}
                >
                  <td class="tabular">
                    <strong>#{row.id}</strong>
                    <small class="table-subtitle">
                      {new Date(row.time).toLocaleString()}
                    </small>
                  </td>
                  <td>
                    <Badge
                      tone={
                        row.action === "DELETE"
                          ? "red"
                          : row.action === "UPDATE"
                            ? "orange"
                            : "green"
                      }
                    >
                      {row.action}
                    </Badge>
                  </td>
                  <td>
                    <strong>{row.entity}</strong>
                    <small class="table-subtitle mono">{row.entity_id}</small>
                  </td>
                  <td class="text-xs">{actor(row.actor)}</td>
                  <td>{row.operation}</td>
                  <td>
                    <Badge>{row.transport}</Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!page.loading && !page.data?.length && (
          <Empty
            title="No matching mutations"
            text="Confirmed changes appear here; reads and rolled-back writes do not."
          />
        )}
        <div class="table-footer">
          <span>Page {cursors.length + 1} · 100 entries per page</span>
          <div class="flex gap-2">
            <button
              class="button-secondary"
              disabled={!cursors.length || page.loading}
              onClick={() => setCursors((c) => c.slice(0, -1))}
            >
              Previous
            </button>
            <button
              class="button-secondary"
              disabled={page.data?.length !== 100 || page.loading}
              onClick={() => setCursors((c) => [...c, page.data!.at(-1)!.id])}
            >
              Older entries <Icon name="arrow" size={14} />
            </button>
          </div>
        </div>
      </Panel>
      {selected && (
        <Drawer
          title={`Mutation #${selected.id}`}
          onClose={() => setSelected(null)}
        >
          <div class="drawer-meta">
            <Badge tone="orange">{selected.action}</Badge>
            <strong>{selected.entity}</strong>
            <small>{selected.entity_id}</small>
          </div>
          <div class="health-list compact">
            <div>
              <span>Actor</span>
              <strong>{actor(selected.actor)}</strong>
            </div>
            <div>
              <span>Operation</span>
              <strong>{selected.operation}</strong>
            </div>
            <div>
              <span>Transport</span>
              <strong>{selected.transport}</strong>
            </div>
          </div>
          <button
            class="button-secondary"
            onClick={() => onCopy(selected.transaction_id)}
          >
            <Icon name="copy" size={14} />
            Copy transaction ID
          </button>
          <h3 class="mt-6">Before</h3>
          <pre class="code-block">
            {JSON.stringify(selected.before, null, 2)}
          </pre>
          <h3>After</h3>
          <pre class="code-block">
            {JSON.stringify(selected.after, null, 2)}
          </pre>
          <p class="muted text-xs">
            Entity snapshots preserve SQL representations; binary file entries
            show byte counts.
          </p>
        </Drawer>
      )}
    </>
  );
}
