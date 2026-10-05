import { useEffect, useState } from "preact/hooks";
import { Badge, Drawer, Empty, ErrorBanner, Icon, Panel } from "./components";
import { useDebounce, usePolling } from "./hooks";
import type { Api } from "./types";
type Table = {
  name: string;
  stream: boolean;
  fields: {
    name: string;
    type: string;
    relation: unknown;
    generated: boolean;
    unique: boolean;
  }[];
};
type Row = {
  record: Record<string, unknown>;
  etag: string;
  oversized: boolean;
};
export function DatabasePage({ api }: { api: Api }) {
  const tables = usePolling<Table[]>(api, "/api/data");
  const [entity, setEntity] = useState(""),
    [search, setSearch] = useState(""),
    [cursors, setCursors] = useState<number[]>([]),
    [selected, setSelected] = useState<Row | null>(null),
    [creating, setCreating] = useState(false),
    [editor, setEditor] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false),
    [confirmDelete, setConfirmDelete] = useState(false);
  const debounced = useDebounce(search);
  useEffect(() => {
    if (!entity && tables.data?.length) setEntity(tables.data[0].name);
  }, [tables.data]);
  useEffect(() => setCursors([]), [entity, debounced]);
  const query = new URLSearchParams({
    entity,
    search: debounced,
    after: String(cursors.at(-1) ?? 0),
  });
  const rows = usePolling<{ rows: Row[]; next_cursor: number | null }>(
    api,
    `/api/data/rows?${query}`,
  );
  const table = tables.data?.find((t) => t.name === entity);
  const columns = table?.fields.filter((f) => !f.relation) ?? [];
  const open = (row: Row | null) => {
    setSelected(row);
    setCreating(!row);
    setError("");
    setConfirmDelete(false);
    setEditor(
      JSON.stringify(
        row?.record ??
          Object.fromEntries(
            columns
              .filter((f) => !f.generated && f.name !== "id")
              .map((f) => [
                f.name,
                f.type.startsWith("Optional")
                  ? null
                  : f.type === "Bool"
                    ? false
                    : f.type.startsWith("List")
                      ? []
                      : f.type.startsWith("Number") ||
                          f.type.startsWith("Id(") ||
                          f.type.startsWith("Named(")
                        ? 1
                        : "",
              ]),
          ),
        null,
        2,
      ),
    );
  };
  const write = async (action: string) => {
    setBusy(true);
    setError("");
    try {
      await api("/api/data", {
        method: "POST",
        body: JSON.stringify({
          action,
          entity,
          id: selected?.record.id,
          etag: selected?.etag,
          record: action === "DELETE" ? null : JSON.parse(editor),
        }),
      });
      setCreating(false);
      setSelected(null);
      rows.refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Write failed");
    } finally {
      setBusy(false);
    }
  };
  const reloadRecord = async () => {
    if (!selected) return;
    setBusy(true);
    setError("");
    try {
      const page = await api<{ rows: Row[] }>(
        `/api/data/rows?${new URLSearchParams({ entity, id: String(selected.record.id) })}`,
      );
      const latest = page.rows[0];
      if (!latest || latest.oversized)
        throw Error("Record is unavailable or exceeds the browser limit");
      open(latest);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Reload failed");
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <div class="info-callout">
        <Icon name="audit" />
        <div>
          <strong>Privileged database access.</strong> Changes bypass
          application permissions, validate data and references, and commit with
          an admin audit record. Normal HTTP console permissions still apply.
        </div>
      </div>
      <Panel
        title="Database browser"
        description="Application entities only · bounded pages · search across stored fields"
      >
        <div class="filter-bar">
          <select
            aria-label="Database entity"
            value={entity}
            onChange={(e) => setEntity(e.currentTarget.value)}
          >
            {tables.data?.map((t) => (
              <option key={t.name}>{t.name}</option>
            ))}
          </select>
          <input
            aria-label="Search database records"
            placeholder="Search records…"
            value={search}
            onInput={(e) => setSearch(e.currentTarget.value)}
          />
          <button class="button-secondary" onClick={() => rows.refresh()}>
            Refresh records
          </button>
          <button
            class="button-primary ml-auto"
            onClick={() => open(null)}
            disabled={!entity}
          >
            Create record
          </button>
        </div>
        <ErrorBanner message={tables.error || (entity ? rows.error : "")} />
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                {columns.map((f) => (
                  <th key={f.name} title={f.type}>
                    {f.name}
                  </th>
                ))}
                <th>Action</th>
              </tr>
            </thead>
            <tbody>
              {rows.data?.rows.map((row) => (
                <tr key={String(row.record.id)}>
                  {columns.map((f) => (
                    <td key={f.name} class="mono">
                      <span
                        class="db-cell"
                        title={JSON.stringify(row.record[f.name])}
                      >
                        {typeof row.record[f.name] === "string"
                          ? String(row.record[f.name])
                          : JSON.stringify(row.record[f.name])}
                      </span>
                    </td>
                  ))}
                  <td>
                    {row.oversized ? (
                      <Badge>Record exceeds browser limit</Badge>
                    ) : (
                      <button
                        class="button-secondary"
                        aria-label={`Edit record ${row.record.id}`}
                        onClick={() => open(row)}
                      >
                        Edit
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!rows.data?.rows.length && (
          <Empty
            icon="resources"
            title="No records"
            text="Change the search or create a record."
          />
        )}
        <div class="table-footer">
          <span>
            Page {cursors.length + 1} · up to 50 records · ascending ID
          </span>
          <div class="flex gap-2">
            <button
              class="button-secondary"
              disabled={!cursors.length}
              onClick={() => setCursors((c) => c.slice(0, -1))}
            >
              Previous
            </button>
            <button
              class="button-secondary"
              disabled={!rows.data?.next_cursor}
              onClick={() => setCursors((c) => [...c, rows.data!.next_cursor!])}
            >
              Next records
            </button>
          </div>
        </div>
      </Panel>
      {(creating || selected) && (
        <Drawer
          title={`${creating ? "Create" : "Edit"} ${entity}`}
          onClose={() => {
            setCreating(false);
            setSelected(null);
          }}
        >
          <p class="muted">
            Primary keys are immutable. Omit the ID when creating a record to
            allocate it automatically.
          </p>
          <label htmlFor="db-record">Record JSON</label>
          <textarea
            id="db-record"
            class="code-editor db-editor w-full"
            value={editor}
            onInput={(e) => setEditor(e.currentTarget.value)}
            spellcheck={false}
          />
          <ErrorBanner message={error} />
          <div class="flex gap-2 mt-5">
            <button
              class="button-primary"
              disabled={busy}
              onClick={() => write(creating ? "CREATE" : "UPDATE")}
            >
              {busy ? "Saving…" : "Save record"}
            </button>
            {selected && (
              <button
                class="button-secondary"
                disabled={busy}
                onClick={reloadRecord}
              >
                Reload record
              </button>
            )}
            {selected && (
              <button
                class="button-secondary"
                disabled={busy}
                onClick={() => setConfirmDelete(true)}
              >
                Delete record
              </button>
            )}
          </div>
          {confirmDelete && (
            <div class="info-callout mt-5">
              <div>
                <strong>Delete this record permanently?</strong>
                <p>
                  References can prevent deletion. The deletion will be audited.
                </p>
                <button
                  class="button-primary mt-3"
                  disabled={busy}
                  onClick={() => write("DELETE")}
                >
                  Confirm deletion
                </button>
              </div>
            </div>
          )}
          <details class="response-headers mt-5">
            <summary>Entity schema</summary>
            <pre>{JSON.stringify(table, null, 2)}</pre>
          </details>
        </Drawer>
      )}
    </>
  );
}
