import { useEffect, useLayoutEffect, useState } from "preact/hooks";
import { Badge, Empty, ErrorBanner, Icon, Panel } from "./components";
import { duration, formatNumber } from "./data";
import { usePolling } from "./hooks";
import type { Adapter, Api, IssuedToken, Overview } from "./types";
export function ConsolePage({
  data,
  adminToken,
  authorization,
  setAuthorization,
  initialEndpoint,
  onJwt,
  onCopy,
}: {
  data: Overview;
  adminToken: string;
  authorization: string;
  setAuthorization: (value: string) => void;
  initialEndpoint: string;
  onJwt: () => void;
  onCopy: (text: string) => void;
}) {
  const endpoints = data.endpoints.filter((e) => e.method !== "WS");
  const [name, setName] = useState(initialEndpoint || endpoints[0]?.name || ""),
    [path, setPath] = useState(""),
    [body, setBody] = useState(""),
    [result, setResult] = useState<{
      status: number;
      headers: Record<string, string>;
      body: string;
      elapsed: number;
    } | null>(null),
    [loading, setLoading] = useState(false),
    [error, setError] = useState("");
  const op = endpoints.find((e) => e.name === name);
  useLayoutEffect(() => {
    if (!op) return;
    setPath(op.path);
    const fields = op.inputs.filter((i) => !op.path.includes(`{${i.name}}`));
    setBody(
      fields.length
        ? JSON.stringify(
            Object.fromEntries(
              fields.map((i) => [
                i.name,
                i.default ??
                  (i.type.includes("List")
                    ? []
                    : i.type === "Bool"
                      ? false
                      : i.type.includes("Number") || i.type.startsWith("Id(")
                        ? 1
                        : ""),
              ]),
            ),
            null,
            2,
          )
        : "",
    );
    setResult(null);
    setError("");
  }, [name]);
  const send = async () => {
    setLoading(true);
    setError("");
    const start = performance.now();
    try {
      const payload = body.trim() ? JSON.parse(body) : null;
      const response = await fetch("/api/call", {
        method: "POST",
        headers: {
          Authorization: `Bearer ${adminToken}`,
          "Content-Type": "application/json",
        },
        body: JSON.stringify({
          endpoint: name,
          path,
          authorization,
          body: payload,
        }),
      });
      const type = response.headers.get("content-type") ?? "";
      let content = type.startsWith("image/")
        ? `Binary response: ${(await response.arrayBuffer()).byteLength} bytes (${type})`
        : await response.text();
      try {
        content = JSON.stringify(JSON.parse(content), null, 2);
      } catch {}
      setResult({
        status: response.status,
        headers: Object.fromEntries(response.headers),
        body: content.slice(0, 128 * 1024),
        elapsed: performance.now() - start,
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Request failed");
    } finally {
      setLoading(false);
    }
  };
  return (
    <>
      <div class="info-callout">
        <Icon name="console" />
        <div>
          <strong>A real request, with real permissions.</strong>
          <span>
            {" "}
            The console calls local application endpoints. Mutations change data
            and create audit records.
          </span>
        </div>
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-[1.1fr_1fr] gap-5">
        <Panel
          title="Request builder"
          description="Exercise your declared HTTP API"
        >
          <div class="form-body">
            <label htmlFor="operation">Operation</label>
            <select
              id="operation"
              value={name}
              onChange={(e) => setName(e.currentTarget.value)}
            >
              {endpoints.map((e) => (
                <option value={e.name} key={e.name}>
                  {e.method} {e.path} · {e.name}
                </option>
              ))}
            </select>
            <label htmlFor="request-path">Request path</label>
            <div class="request-path">
              <Badge tone={op?.method === "GET" ? "green" : "orange"}>
                {op?.method}
              </Badge>
              <input
                id="request-path"
                value={path}
                onInput={(e) => setPath(e.currentTarget.value)}
                spellcheck={false}
              />
            </div>
            <p class="field-note">
              Replace path parameters. Query strings are supported.
            </p>
            <div class="flex items-center justify-between mt-5">
              <label htmlFor="request-auth" class="!m-0">
                Application authorization
              </label>
              <button class="text-button" onClick={onJwt}>
                <Icon name="key" size={14} />
                Generate JWT
              </button>
            </div>
            <input
              id="request-auth"
              type="password"
              value={authorization}
              onInput={(e) => setAuthorization(e.currentTarget.value)}
              placeholder="Bearer JWT / ApiKey credential · empty = anonymous"
              autoComplete="off"
            />
            <label htmlFor="request-body">JSON body</label>
            <textarea
              id="request-body"
              class="code-editor"
              value={body}
              onInput={(e) => setBody(e.currentTarget.value)}
              placeholder="No request body"
              spellcheck={false}
            />
            <ErrorBanner message={error} />
            <div class="flex justify-between items-center mt-4">
              <span class="muted text-xs">
                {op?.mutation
                  ? "Mutation · production data will change"
                  : "Query · normal application authorization"}
              </span>
              <button
                class="button-primary"
                disabled={loading || !op}
                onClick={send}
              >
                <Icon name="play" size={15} />
                {loading ? "Sending…" : "Send request"}
              </button>
            </div>
          </div>
        </Panel>
        <Panel
          title="Response"
          description="Status, timing and response headers"
          action={
            result && (
              <Badge tone={result.status >= 400 ? "red" : "green"}>
                HTTP {result.status}
              </Badge>
            )
          }
        >
          {result ? (
            <>
              <div class="response-toolbar">
                <span>
                  {formatNumber(result.elapsed, 2)} ms <small>end-to-end</small>
                </span>
                <button class="text-button" onClick={() => onCopy(result.body)}>
                  <Icon name="copy" size={14} />
                  Copy body
                </button>
              </div>
              <pre class="response-body">{result.body || "Empty response"}</pre>
              <details class="response-headers">
                <summary>Response headers</summary>
                <pre>{JSON.stringify(result.headers, null, 2)}</pre>
              </details>
            </>
          ) : (
            <Empty
              title="Ready when you are"
              text="Send a request to inspect the response and headers."
            />
          )}
        </Panel>
      </div>
      <Panel
        title="Input contract"
        description={op ? `Typed inputs for ${op.name}` : "Select an operation"}
      >
        <div class="table-wrap">
          <table>
            <thead>
              <tr>
                <th>Field</th>
                <th>Type</th>
                <th>Default</th>
                <th>Location</th>
              </tr>
            </thead>
            <tbody>
              {op?.inputs.map((i) => (
                <tr key={i.name}>
                  <td class="mono">{i.name}</td>
                  <td class="mono text-xs">{i.type}</td>
                  <td class="mono">
                    {i.default != null ? JSON.stringify(i.default) : "—"}
                  </td>
                  <td>
                    <Badge>
                      {op.path.includes(`{${i.name}}`)
                        ? "Path"
                        : "JSON / query"}
                    </Badge>
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        {!op?.inputs.length && (
          <div class="panel-note">This endpoint requires no input fields.</div>
        )}
      </Panel>
    </>
  );
}
export function TokensPage({
  api,
  onUse,
  onCopy,
}: {
  api: Api;
  onUse: (authorization: string) => void;
  onCopy: (text: string) => void;
}) {
  const adapters = usePolling<Adapter[]>(api, "/api/jwt"),
    [alias, setAlias] = useState(""),
    [subject, setSubject] = useState(""),
    [ttl, setTtl] = useState("3600"),
    [issued, setIssued] = useState<IssuedToken | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(false),
    [show, setShow] = useState(false);
  useEffect(() => {
    if (adapters.data?.length && !alias) setAlias(adapters.data[0].alias);
  }, [adapters.data]);
  const adapter = adapters.data?.find((a) => a.alias === alias);
  useEffect(() => {
    setIssued(null);
  }, [alias]);
  const generate = async () => {
    setLoading(true);
    setError("");
    setIssued(null);
    try {
      setIssued(
        await api<IssuedToken>("/api/jwt", {
          method: "POST",
          body: JSON.stringify({
            adapter: alias,
            subject,
            ttl_seconds: Number(ttl),
          }),
        }),
      );
      setShow(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Token generation failed");
    } finally {
      setLoading(false);
    }
  };
  return (
    <>
      <div class="info-callout">
        <Icon name="key" />
        <div>
          <strong>Issue a token for an existing application identity.</strong>
          <span>
            {" "}
            The server signs HS256 tokens using configured adapter keys. Signing
            keys never leave the runtime.
          </span>
        </div>
      </div>
      <div class="grid grid-cols-1 xl:grid-cols-2 gap-5">
        <Panel
          title="JWT token issuer"
          description="Admin-only · fixed issuer and audience · bounded lifetime"
        >
          <div class="form-body">
            <ErrorBanner message={adapters.error} />
            <label htmlFor="jwt-adapter">Authentication adapter</label>
            <select
              id="jwt-adapter"
              value={alias}
              onChange={(e) => setAlias(e.currentTarget.value)}
            >
              {adapters.data?.map((a) => (
                <option key={a.alias} value={a.alias}>
                  {a.alias}
                  {!a.configured ? " · key not configured" : ""}
                </option>
              ))}
            </select>
            <label htmlFor="jwt-subject">Subject</label>
            <input
              id="jwt-subject"
              value={subject}
              onInput={(e) => setSubject(e.currentTarget.value)}
              list="jwt-subjects"
              placeholder="Existing identity subject"
              autoComplete="off"
            />
            <datalist id="jwt-subjects">
              {adapter?.subjects.map((s) => (
                <option key={s.subject} value={s.subject}>
                  {s.actor_id}
                </option>
              ))}
            </datalist>
            <p class="field-note">
              Subjects resolve to real actors. Roles and permissions come from
              the database, not token claims.
            </p>
            <label htmlFor="jwt-ttl">Token lifetime</label>
            <select
              id="jwt-ttl"
              value={ttl}
              onChange={(e) => setTtl(e.currentTarget.value)}
            >
              <option value="300">5 minutes</option>
              <option value="900">15 minutes</option>
              <option value="3600">1 hour</option>
              <option value="14400">4 hours</option>
              <option value="86400">24 hours</option>
            </select>
            <div class="identity-contract">
              <div>
                <span>Algorithm</span>
                <strong>HS256</strong>
              </div>
              <div>
                <span>Issuer</span>
                <code>{adapter?.issuer ?? "—"}</code>
              </div>
              <div>
                <span>Audience</span>
                <code>{adapter?.audience ?? "—"}</code>
              </div>
            </div>
            <ErrorBanner message={error} />
            <button
              class="button-primary w-full mt-5"
              disabled={loading || !adapter?.configured || !subject}
              onClick={generate}
            >
              <Icon name="key" size={16} />
              {loading ? "Signing…" : "Generate application token"}
            </button>
          </div>
        </Panel>
        <Panel
          title="Issued credential"
          description="Kept in this page's memory; never saved to browser storage"
          action={issued && <Badge tone="green">Ready</Badge>}
        >
          {issued ? (
            <div class="form-body">
              <div class="identity-contract">
                <div>
                  <span>Subject</span>
                  <strong>{issued.claims.sub}</strong>
                </div>
                <div>
                  <span>Expires</span>
                  <strong>
                    {new Date(issued.claims.exp * 1000).toLocaleString()}
                  </strong>
                </div>
                <div>
                  <span>Lifetime</span>
                  <strong>{duration(Number(ttl))}</strong>
                </div>
              </div>
              <label htmlFor="issued-jwt">Bearer credential</label>
              <textarea
                id="issued-jwt"
                readOnly
                class="code-editor token-editor"
                value={
                  show
                    ? issued.authorization
                    : "Token generated. Use the buttons below to copy it or attach it to the HTTP console."
                }
              />
              <div class="flex gap-2 flex-wrap mt-3">
                <button class="button-secondary" onClick={() => setShow(!show)}>
                  {show ? "Hide token" : "Reveal token"}
                </button>
                <button
                  class="button-secondary"
                  onClick={() => onCopy(issued.authorization)}
                >
                  <Icon name="copy" size={14} />
                  Copy credential
                </button>
                <button
                  class="button-primary"
                  onClick={() => onUse(issued.authorization)}
                >
                  Use in console <Icon name="arrow" size={14} />
                </button>
              </div>
              <details class="response-headers mt-5">
                <summary>Inspect JWT claims</summary>
                <pre>{JSON.stringify(issued.claims, null, 2)}</pre>
              </details>
            </div>
          ) : (
            <Empty
              title="No token issued"
              text="Choose an adapter and an existing subject to generate a credential."
            />
          )}
        </Panel>
      </div>
    </>
  );
}
