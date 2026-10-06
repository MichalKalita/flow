import { useState } from "preact/hooks";
import { ErrorBanner, Panel } from "./components";
import { usePolling } from "./hooks";
import type { Api } from "./types";

type Backup = {
  id: string;
  created_at: string;
  projects: string[];
  bytes: number;
};
export function BackupsPage({ api }: { api: Api }) {
  const backups = usePolling<{ backups: Backup[] }>(api, "/api/backups", 10000);
  const [busy, setBusy] = useState(false);
  const [password, setPassword] = useState("");
  const [selected, setSelected] = useState("");
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const create = async () => {
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await api("/api/backups", { method: "POST" });
      backups.refresh();
      setNotice("Backup saved.");
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to create backup.",
      );
    } finally {
      setBusy(false);
    }
  };
  const verify = async (event: Event) => {
    event.preventDefault();
    setBusy(true);
    setError("");
    setNotice("");
    try {
      await api("/api/backups/verify", {
        method: "POST",
        body: JSON.stringify({ id: selected, password }),
      });
      setPassword("");
      setNotice(
        "Backup restored and verified in a separate temporary copy. Your running data is unchanged.",
      );
    } catch (error) {
      setError(
        error instanceof Error ? error.message : "Unable to verify backup.",
      );
    } finally {
      setBusy(false);
    }
  };
  const restore = async () => {
    const backup = backups.data?.backups.find(
      (backup) => backup.id === selected,
    );
    if (
      !backup ||
      !confirm(
        `Restore all applications to ${new Date(backup.created_at).toLocaleString()}? Current data will be kept in a recovery copy.`,
      )
    )
      return;
    setBusy(true);
    setError("");
    setNotice("Restoring your server…");
    try {
      const result = await api<{ request: string }>("/api/backups/restore", {
        method: "POST",
        body: JSON.stringify({ id: selected, password, confirm: true }),
      });
      let restored = false;
      for (let attempt = 0; attempt < 120; attempt++) {
        await new Promise((resolve) => setTimeout(resolve, 500));
        try {
          const status = await api<{ status: string; request: string }>(
            "/api/backups/status",
          );
          if (
            status.status === "completed" &&
            status.request === result.request
          ) {
            restored = true;
            break;
          }
        } catch {}
      }
      if (!restored)
        throw Error(
          "Restoration is taking longer than expected. Check the server status before trying again.",
        );
      setPassword("");
      setNotice(
        "Server restored. A recovery copy of the previous data was kept.",
      );
      backups.refresh();
    } catch (error) {
      setError(
        error instanceof Error
          ? error.message
          : "Unable to restore the server.",
      );
    } finally {
      setBusy(false);
    }
  };
  return (
    <>
      <ErrorBanner message={error || backups.error} />
      <Panel
        title="Backups"
        description="Save all your applications and check that they can be restored."
      >
        <button class="button-primary" disabled={busy} onClick={create}>
          {busy ? "Working…" : "Create backup"}
        </button>
        {notice && <p role="status">{notice}</p>}
        <div class="backup-list">
          {backups.data?.backups.map((backup) => (
            <article key={backup.id}>
              <div>
                <strong>{new Date(backup.created_at).toLocaleString()}</strong>
                <p>
                  {backup.projects.length} applications ·{" "}
                  {(backup.bytes / 1048576).toFixed(1)} MB
                </p>
              </div>
              <button
                class="button-secondary"
                disabled={busy}
                onClick={() => {
                  setSelected(backup.id);
                  setNotice("");
                }}
              >
                Check restore
              </button>
            </article>
          ))}
        </div>
        {!backups.data?.backups.length && (
          <p>No backups yet. Create the first one now.</p>
        )}
      </Panel>
      {selected && (
        <Panel
          title="Check this backup"
          description="Use the owner password from when this backup was created to recover its encryption keys."
        >
          <form class="server-form" onSubmit={verify}>
            <label htmlFor="recovery-password">Recovery password</label>
            <input
              id="recovery-password"
              type="password"
              value={password}
              onInput={(event) => setPassword(event.currentTarget.value)}
              required
              autoComplete="current-password"
            />
            <button class="button-primary" disabled={busy}>
              {busy ? "Checking…" : "Verify restoration"}
            </button>
            <button
              type="button"
              class="button-secondary"
              disabled={busy || !password}
              onClick={restore}
            >
              Restore this backup
            </button>
          </form>
        </Panel>
      )}
    </>
  );
}
