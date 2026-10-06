import { render } from "preact";
import { useEffect, useState } from "preact/hooks";

type Field = {
  name: string;
  label: string;
  kind: string;
  required?: boolean;
  max_bytes?: number;
};
type Metadata = {
  title: string;
  item_label: string;
  id_input: string;
  fields: Field[];
  operations: Record<string, { method: string; path: string }>;
};
type RecordRow = {
  id: number;
  version: number;
  [name: string]: string | number;
};
const base = location.pathname.replace(/\/app$/, "");
const key = `flow.application.${base}`;
function savedCredential(): string {
  try {
    return sessionStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}
function App() {
  const [authorization, setAuthorization] = useState(savedCredential);
  const [access, setAccess] = useState("");
  const [metadata, setMetadata] = useState<Metadata | null>(null);
  const [rows, setRows] = useState<RecordRow[]>([]);
  const [values, setValues] = useState<Record<string, string>>({});
  const [editing, setEditing] = useState<RecordRow | null>(null);
  const [busy, setBusy] = useState(false);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [more, setMore] = useState(false);
  const remember = (value: string) => {
    setAuthorization(value);
    try {
      if (value) sessionStorage.setItem(key, value);
      else sessionStorage.removeItem(key);
    } catch {}
  };
  useEffect(() => {
    const listener = (event: MessageEvent) => {
      if (
        event.source !== window.opener ||
        event.data?.type !== "flow.application.auth" ||
        typeof event.data.authorization !== "string"
      )
        return;
      remember(event.data.authorization);
    };
    window.addEventListener("message", listener);
    const ready = () => {
      if (!authorization)
        window.opener?.postMessage({ type: "flow.application.ready" }, "*");
    };
    ready();
    const timer = setInterval(ready, 500);
    return () => {
      window.removeEventListener("message", listener);
      clearInterval(timer);
    };
  }, [authorization]);
  const call = async (
    operation: { method: string; path: string },
    input?: Record<string, unknown>,
  ) => {
    let path = operation.path;
    const body = { ...input };
    for (const match of path.matchAll(/\{([^}]+)\}/g)) {
      path = path.replace(match[0], encodeURIComponent(String(body[match[1]])));
      delete body[match[1]];
    }
    if (operation.method === "GET") {
      const query = new URLSearchParams(
        Object.entries(body)
          .filter(([, value]) => value !== null && value !== undefined)
          .map(([name, value]) => [name, String(value)]),
      );
      if (query.size) path += `?${query}`;
    }
    const response = await fetch(`${base}${path}`, {
      method: operation.method,
      headers: {
        Authorization: authorization,
        "Content-Type": "application/json",
      },
      body: operation.method === "GET" ? undefined : JSON.stringify(body),
    });
    if (!response.ok) {
      if (response.status === 401) {
        remember("");
        throw Error(
          "Your session ended. Open the application again or sign in.",
        );
      }
      if (response.status === 409)
        throw Error("This record changed. Refresh before editing it again.");
      if (response.status === 403 || response.status === 404)
        throw Error(
          "This action is unavailable with your current permissions.",
        );
      throw Error("Check your entries and try again.");
    }
    return response.json();
  };
  const load = async (configuration: Metadata, append = false) => {
    setLoading(true);
    setError("");
    try {
      const after = append ? rows.at(-1)?.id : undefined;
      const result: RecordRow[] = await call(configuration.operations.list, {
        after,
      });
      setRows(result);
      setMore(result.length === 50);
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to load records.",
      );
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => {
    if (!authorization) {
      setMetadata(null);
      setRows([]);
      return;
    }
    let cancelled = false;
    fetch(`${base}/app/config`, { headers: { Authorization: authorization } })
      .then(async (response) => {
        if (!response.ok)
          throw Error(
            "This access code is not valid. Open the application from Flow.",
          );
        const value: Metadata = await response.json();
        if (cancelled) return;
        setMetadata(value);
        document.title = value.title;
        setValues(
          Object.fromEntries(value.fields.map((field) => [field.name, ""])),
        );
        await load(value);
      })
      .catch((error) => {
        if (!cancelled) {
          remember("");
          setError(error.message);
        }
      });
    return () => {
      cancelled = true;
    };
  }, [authorization]);
  const edit = (row: RecordRow | null) => {
    if (!metadata) return;
    setEditing(row);
    setValues(
      Object.fromEntries(
        metadata.fields.map((field) => [
          field.name,
          String(row?.[field.name] ?? ""),
        ]),
      ),
    );
    setError("");
    setNotice("");
  };
  const save = async (event: Event) => {
    event.preventDefault();
    if (!metadata) return;
    for (const field of metadata.fields) {
      if (field.required && !(values[field.name] ?? "").trim()) {
        setError(`Enter ${field.label.toLowerCase()}.`);
        return;
      }
      if (
        field.max_bytes &&
        new TextEncoder().encode(values[field.name] ?? "").length >
          field.max_bytes
      ) {
        setError(`${field.label} is too long.`);
        return;
      }
    }
    setBusy(true);
    setError("");
    try {
      await call(
        editing ? metadata.operations.update : metadata.operations.create,
        {
          ...values,
          ...(editing
            ? { [metadata.id_input]: editing.id, version: editing.version }
            : {}),
        },
      );
      edit(null);
      setNotice(editing ? "Record updated." : "Record added.");
      await load(metadata);
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to save record.",
      );
    } finally {
      setBusy(false);
    }
  };
  const remove = async (row: RecordRow) => {
    if (
      !metadata ||
      !confirm(`Delete ${row[metadata.fields[0].name] ?? "this record"}?`)
    )
      return;
    setBusy(true);
    setError("");
    try {
      await call(metadata.operations.delete, {
        [metadata.id_input]: row.id,
        version: row.version,
      });
      if (editing?.id === row.id) edit(null);
      setNotice("Record deleted.");
      await load(metadata);
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to delete record.",
      );
    } finally {
      setBusy(false);
    }
  };
  if (!authorization || !metadata)
    return (
      <main class="signin">
        <h1>
          {window.opener && !authorization
            ? "Opening your application…"
            : "Sign in"}
        </h1>
        {error && <p role="alert">{error}</p>}
        <form
          onSubmit={(event) => {
            event.preventDefault();
            remember(
              access.startsWith("Bearer ") ? access : `Bearer ${access}`,
            );
            setAccess("");
          }}
        >
          <label htmlFor="access">Access code</label>
          <input
            id="access"
            type="password"
            value={access}
            onInput={(event) => setAccess(event.currentTarget.value)}
            required
            autoComplete="off"
          />
          <button>Sign in</button>
        </form>
        <p>
          Open this application from your Flow server to sign in automatically.
        </p>
      </main>
    );
  return (
    <div class="application">
      <header>
        <div>
          <small>Your application</small>
          <h1>{metadata.title}</h1>
        </div>
        <button class="secondary" onClick={() => remember("")}>
          Sign out
        </button>
      </header>
      <main>
        <section class="editor">
          <h2>
            {editing
              ? `Edit ${metadata.item_label}`
              : `Add a ${metadata.item_label}`}
          </h2>
          <form onSubmit={save}>
            {metadata.fields.map((field) => (
              <div key={field.name}>
                <label htmlFor={`field-${field.name}`}>{field.label}</label>
                {field.kind === "textarea" ? (
                  <textarea
                    id={`field-${field.name}`}
                    value={values[field.name] ?? ""}
                    onInput={(event) =>
                      setValues((current) => ({
                        ...current,
                        [field.name]: event.currentTarget.value,
                      }))
                    }
                    rows={4}
                  />
                ) : (
                  <input
                    id={`field-${field.name}`}
                    type={field.kind}
                    value={values[field.name] ?? ""}
                    onInput={(event) =>
                      setValues((current) => ({
                        ...current,
                        [field.name]: event.currentTarget.value,
                      }))
                    }
                    required={field.required}
                  />
                )}
              </div>
            ))}
            <div class="actions">
              <button disabled={busy}>
                {busy
                  ? "Saving…"
                  : editing
                    ? "Save changes"
                    : `Add ${metadata.item_label}`}
              </button>
              {editing && (
                <button
                  type="button"
                  class="secondary"
                  onClick={() => edit(null)}
                >
                  Cancel
                </button>
              )}
            </div>
          </form>
        </section>
        <section class="collection">
          <div class="section-header">
            <h2>{metadata.title}</h2>
            <button
              class="secondary"
              onClick={() => load(metadata)}
              disabled={loading}
            >
              Refresh
            </button>
          </div>
          {error && (
            <p role="alert" class="error">
              {error}
            </p>
          )}
          {notice && <p role="status">{notice}</p>}
          {!rows.length && (
            <p>
              {loading
                ? "Loading…"
                : `No records yet. Add your first ${metadata.item_label}.`}
            </p>
          )}
          <div class="contact-list">
            {rows.map((row) => (
              <article key={row.id}>
                <div>
                  <h3>{row[metadata.fields[0].name]}</h3>
                  {metadata.fields
                    .filter(
                      (field) =>
                        field.name !== metadata.fields[0].name &&
                        row[field.name],
                    )
                    .map((field) => (
                      <p key={field.name}>{row[field.name]}</p>
                    ))}
                </div>
                <div class="actions">
                  <button class="secondary" onClick={() => edit(row)}>
                    Edit
                  </button>
                  <button
                    class="danger"
                    disabled={busy}
                    onClick={() => remove(row)}
                  >
                    Delete
                  </button>
                </div>
              </article>
            ))}
          </div>
          {more && (
            <button
              class="secondary"
              disabled={loading}
              onClick={() => load(metadata, true)}
            >
              Next page
            </button>
          )}
        </section>
      </main>
      <footer>Saved on your Flow server.</footer>
    </div>
  );
}
render(<App />, document.getElementById("app")!);
