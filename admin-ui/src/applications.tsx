import { useRef, useState } from "preact/hooks";
import { Empty, ErrorBanner, Panel } from "./components";
import { usePolling } from "./hooks";
import type { Api } from "./types";

type Launch = {
  name: string;
  base_path: string;
  authorization: string;
  public_port: number | null;
};
type Catalog = {
  templates: { id: string; name: string; description: string }[];
  installed: { name: string; title: string; base_path: string }[];
};

export function ApplicationsPage({ api }: { api: Api }) {
  const catalog = usePolling<Catalog>(api, "/api/catalog", 5000);
  const opening = useRef(false);
  const requests = useRef<Record<string, string>>({});
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const open = async (template?: string, project?: string) => {
    if (opening.current) return;
    opening.current = true;
    const popup = window.open("about:blank", "_blank");
    setBusy(template ?? project ?? "");
    setError("");
    try {
      const launch = await api<Launch>(
        template ? "/api/catalog" : "/api/catalog/launch",
        {
          method: "POST",
          body: JSON.stringify(
            template
              ? {
                  template,
                  request_id:
                    requests.current[template] ??
                    (requests.current[template] = installationId()),
                }
              : { project },
          ),
        },
      );
      if (template) delete requests.current[template];
      catalog.refresh();
      if (!popup)
        throw Error(
          "Application ready. Allow popups and choose Open to use it.",
        );
      const url = new URL(
        launch.base_path,
        `http://${location.hostname.includes(":") ? `[${location.hostname}]` : location.hostname}:${launch.public_port ?? 80}`,
      );
      const listener = (event: MessageEvent) => {
        if (
          event.source !== popup ||
          event.origin !== url.origin ||
          event.data?.type !== "flow.application.ready"
        )
          return;
        popup.postMessage(
          {
            type: "flow.application.auth",
            authorization: launch.authorization,
          },
          url.origin,
        );
        window.removeEventListener("message", listener);
      };
      window.addEventListener("message", listener);
      setTimeout(() => window.removeEventListener("message", listener), 30000);
      popup.location.href = url.toString();
    } catch (error) {
      popup?.close();
      setError(
        error instanceof Error ? error.message : "Unable to open application.",
      );
    } finally {
      setBusy("");
      opening.current = false;
    }
  };
  return (
    <>
      <ErrorBanner message={error || catalog.error} />
      <Panel
        title="Your applications"
        description="Open an application and get to work."
      >
        {!catalog.data?.installed.length && (
          <Empty
            title="Choose your first application"
            text="Use a ready-made tool from the catalog below."
          />
        )}
        <div class="catalog-grid">
          {catalog.data?.installed.map((application) => (
            <article class="catalog-card" key={application.name}>
              <h3>{application.title}</h3>
              <p>{application.name}</p>
              <button
                class="button-primary"
                disabled={!!busy}
                onClick={() => open(undefined, application.name)}
              >
                {busy === application.name ? "Opening…" : "Open"}
              </button>
            </article>
          ))}
        </div>
      </Panel>
      <Panel
        title="Catalog"
        description="One click creates your own application. Your data stays separate."
      >
        <div class="catalog-grid">
          {catalog.data?.templates.map((template) => (
            <article class="catalog-card" key={template.id}>
              <h3>{template.name}</h3>
              <p>{template.description}</p>
              <button
                class="button-primary"
                disabled={!!busy}
                onClick={() => open(template.id)}
              >
                {busy === template.id ? "Preparing…" : `Use ${template.name}`}
              </button>
            </article>
          ))}
        </div>
      </Panel>
    </>
  );
}

function installationId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  bytes[6] = (bytes[6] & 15) | 64;
  bytes[8] = (bytes[8] & 63) | 128;
  const value = Array.from(bytes, (byte) =>
    byte.toString(16).padStart(2, "0"),
  ).join("");
  return `${value.slice(0, 8)}-${value.slice(8, 12)}-${value.slice(12, 16)}-${value.slice(16, 20)}-${value.slice(20)}`;
}
