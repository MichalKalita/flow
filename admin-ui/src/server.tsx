import { useEffect, useState } from "preact/hooks";
import { ErrorBanner, Panel } from "./components";
import { usePolling } from "./hooks";
import type { Api } from "./types";

export type ServerSettings = {
  instance: string;
  name: string;
  public_key: string;
  fingerprint: string;
  setup_complete: boolean;
  revision: number;
  backup_keep: number;
  log_target_bytes: number;
  log_chunk_bytes: number;
  logs: { log_disk_bytes: number | null; oldest_log_time: string | null };
  defaults: {
    tokio_workers: number;
    tokio_blocking: number;
    http_admission: number | null;
  };
  secrets: { name: string; preview: string }[];
};

export function Welcome({
  onLogin,
  error: sessionError,
}: {
  onLogin: (token: string) => void;
  error: string;
}) {
  const [status, setStatus] = useState<{
    name: string;
    setup_complete: boolean;
    enrollment_available: boolean;
    password_login: boolean;
  } | null>(null);
  const [name, setName] = useState("");
  const [password, setPassword] = useState("");
  const [tokenMode, setTokenMode] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    fetch("/api/setup")
      .then((response) => response.json())
      .then((value) => {
        setStatus(value);
        setName(value.name ?? "My server");
      })
      .catch(() => setError("Unable to connect to your server."));
  }, []);
  const setup = status && !status.setup_complete && status.enrollment_available;
  const useToken = tokenMode || (status && !setup && !status.password_login);
  const submit = async (event: Event) => {
    event.preventDefault();
    setBusy(true);
    setError("");
    try {
      let token = password;
      if (useToken) {
        const response = await fetch("/api/overview", {
          headers: { Authorization: `Bearer ${token}` },
        });
        if (!response.ok) throw Error("The admin token is not valid.");
      } else {
        const response = await fetch(setup ? "/api/setup" : "/api/login", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({ name, password }),
        });
        const value = await response.json().catch(() => ({}));
        if (!response.ok)
          throw Error(
            value.message ??
              (response.status === 429
                ? "Please wait a minute before trying again."
                : "Unable to sign in."),
          );
        token = value.token;
      }
      setPassword("");
      if (setup) location.hash = "applications";
      onLogin(token);
    } catch (error) {
      setError(error instanceof Error ? error.message : "Unable to connect.");
    } finally {
      setBusy(false);
    }
  };
  return (
    <div class="login-shell">
      <div class="login-brand">
        <span class="brand-mark">
          f<span>.</span>
        </span>
        <span>flow</span>
      </div>
      <div class="login-card">
        <h1>{setup ? "Welcome to your server." : "Welcome back."}</h1>
        <p>
          {setup
            ? "Give it a name and choose your password. We will take care of the settings."
            : "Sign in to manage your applications."}
        </p>
        <form onSubmit={submit}>
          {setup && (
            <>
              <label htmlFor="server-name">Server name</label>
              <input
                id="server-name"
                value={name}
                onInput={(event) => setName(event.currentTarget.value)}
                maxLength={80}
                required
              />
            </>
          )}
          <label htmlFor="owner-password">
            {useToken
              ? "Admin access token"
              : setup
                ? "Choose a password"
                : "Password"}
          </label>
          <input
            id="owner-password"
            type="password"
            value={password}
            onInput={(event) => setPassword(event.currentTarget.value)}
            minLength={setup ? 10 : undefined}
            maxLength={512}
            autoComplete={setup ? "new-password" : "current-password"}
            required
          />
          {setup && (
            <small>
              Use at least 10 characters. Your keys and settings are created
              automatically.
            </small>
          )}
          <ErrorBanner message={error || sessionError} />
          <button class="button-primary w-full" disabled={busy || !status}>
            {busy
              ? "Connecting…"
              : setup
                ? "Start using Flow"
                : useToken
                  ? "Open control plane"
                  : "Sign in"}
          </button>
          {!setup && status?.password_login && (
            <button
              type="button"
              class="button-secondary w-full"
              onClick={() => {
                setTokenMode(!tokenMode);
                setPassword("");
              }}
            >
              {tokenMode ? "Use my password" : "Use an access token"}
            </button>
          )}
        </form>
      </div>
    </div>
  );
}

export function ServerPage({ api }: { api: Api }) {
  const settings = usePolling<ServerSettings>(api, "/api/settings");
  const [name, setName] = useState("");
  const [keep, setKeep] = useState(7);
  const [logTarget, setLogTarget] = useState(1024);
  const [logChunk, setLogChunk] = useState(50);
  const [error, setError] = useState("");
  const [saved, setSaved] = useState(false);
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (settings.data) {
      setName(settings.data.name);
      setKeep(settings.data.backup_keep);
      setLogTarget(settings.data.log_target_bytes / 1048576);
      setLogChunk(settings.data.log_chunk_bytes / 1048576);
    }
  }, [settings.data]);
  const save = async (event: Event) => {
    event.preventDefault();
    if (!settings.data) return;
    setBusy(true);
    setError("");
    setSaved(false);
    try {
      await api("/api/settings", {
        method: "POST",
        body: JSON.stringify({
          revision: settings.data.revision,
          name,
          backup_keep: keep,
          log_target_bytes: Math.round(logTarget * 1048576),
          log_chunk_bytes: Math.round(logChunk * 1048576),
        }),
      });
      settings.refresh();
      setSaved(true);
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to save settings.",
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <ErrorBanner message={error || settings.error} />
      <Panel
        title="Your server"
        description="Good defaults are already in place. Change only what you need."
      >
        <form onSubmit={save} class="server-form">
          <label htmlFor="settings-name">Server name</label>
          <input
            id="settings-name"
            value={name}
            onInput={(event) => setName(event.currentTarget.value)}
            required
            maxLength={80}
          />
          <label htmlFor="backup-keep">Backups to keep</label>
          <input
            id="backup-keep"
            type="number"
            min={1}
            max={100}
            value={keep}
            onInput={(event) => setKeep(Number(event.currentTarget.value))}
          />
          <label htmlFor="log-target">Local log storage (MiB)</label>
          <input
            id="log-target"
            type="number"
            min={1}
            max={102400}
            value={logTarget}
            onInput={(event) => setLogTarget(Number(event.currentTarget.value))}
            required
          />
          <small>
            Older log chunks are removed to stay near this limit. Recommended:
            1024 MiB.
          </small>
          <label htmlFor="log-chunk">Log chunk size (MiB)</label>
          <input
            id="log-chunk"
            type="number"
            min={1}
            max={logTarget}
            value={logChunk}
            onInput={(event) => setLogChunk(Number(event.currentTarget.value))}
            required
          />
          <small>
            Independent of the total storage limit. Recommended: 50 MiB.
          </small>
          <button class="button-primary" disabled={busy || !settings.data}>
            {busy ? "Saving…" : "Save settings"}
          </button>
          {saved && <p role="status">Settings saved.</p>}
        </form>
      </Panel>
      {settings.data && (
        <>
          <Panel
            title="Automatic settings"
            description="Flow adapts to the hardware available to it."
          >
            <dl class="server-details">
              <dt>Server identity</dt>
              <dd>{settings.data.instance}</dd>
              <dt>Local logs</dt>
              <dd>
                {settings.data.logs?.log_disk_bytes == null
                  ? "Unavailable"
                  : `${(settings.data.logs.log_disk_bytes / 1048576).toFixed(1)} MiB`}
              </dd>
              <dt>Oldest local log</dt>
              <dd>
                {settings.data.logs?.oldest_log_time
                  ? `${new Date(settings.data.logs.oldest_log_time).toLocaleString()} (${Math.max(0, Math.floor((Date.now() - Date.parse(settings.data.logs.oldest_log_time)) / 60000))} minutes old)`
                  : "No logs yet"}
              </dd>
              <dt>Workers</dt>
              <dd>{settings.data.defaults.tokio_workers}</dd>
              <dt>Concurrent requests</dt>
              <dd>
                {settings.data.defaults.http_admission ??
                  "Unlimited (explicit override)"}
              </dd>
            </dl>
          </Panel>
          <details>
            <summary>Advanced: peer identity</summary>
            <Panel
              title="Public key"
              description="This key identifies your server. Private keys stay on disk with restricted access."
            >
              <code class="server-key">{settings.data.public_key}</code>
              <p class="mono">{settings.data.fingerprint}</p>
            </Panel>
          </details>
          {settings.data.secrets.length > 0 && (
            <Panel
              title="Stored secrets"
              description="Only the first and last characters are shown."
            >
              <dl class="server-details">
                {settings.data.secrets.map((secret) => (
                  <div key={secret.name}>
                    <dt>{secret.name}</dt>
                    <dd>{secret.preview}</dd>
                  </div>
                ))}
              </dl>
            </Panel>
          )}
        </>
      )}
    </>
  );
}
