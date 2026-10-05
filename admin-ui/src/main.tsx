import { DatabasePage } from "./database";
import { render } from "preact";
import { useCallback, useEffect, useState } from "preact/hooks";
import { Badge, Empty, ErrorBanner, Icon } from "./components";
import { usePolling } from "./hooks";
import {
  OverviewPage,
  ResourcesPage,
  StreamingPage,
  TrafficPage,
} from "./monitoring";
import { AuditPage, LogsPage } from "./logs";
import { ConsolePage, TokensPage } from "./tools";
import type { Api, Overview } from "./types";
const pages = [
  {
    id: "overview",
    title: "Overview",
    description: "Your entire system, at a glance.",
    icon: "overview",
    group: "OBSERVABILITY",
  },
  {
    id: "traffic",
    title: "Traffic & latency",
    description: "Explore endpoint performance and request volume.",
    icon: "traffic",
  },
  {
    id: "streaming",
    title: "Streaming",
    description: "Connections, subscriptions, messages and event processing.",
    icon: "stream",
  },
  {
    id: "resources",
    title: "Runtime & storage",
    description: "Understand the cost of running your service.",
    icon: "resources",
  },
  {
    id: "logs",
    title: "Log explorer",
    description: "Follow a request. Investigate a failure. Go back in time.",
    icon: "logs",
    group: "EXPLORERS",
  },
  {
    id: "audit",
    title: "Mutation audit",
    description: "A durable record of every committed application change.",
    icon: "audit",
  },
  {
    id: "database",
    title: "Database",
    description: "Browse, search and administer application records.",
    icon: "resources",
    group: "DATA MANAGEMENT",
  },
  {
    id: "console",
    title: "HTTP console",
    description: "Build, authenticate and send application requests.",
    icon: "console",
    group: "DEVELOPER TOOLS",
  },
  {
    id: "tokens",
    title: "Access tokens",
    description: "Generate short-lived JWT credentials for your application.",
    icon: "key",
  },
];
const currentPage = () =>
  pages.some((p) => p.id === location.hash.slice(1))
    ? location.hash.slice(1)
    : "overview";
const ADMIN_TOKEN_KEY = "flow.adminToken";
function readAdminToken(): string {
  try {
    return sessionStorage.getItem(ADMIN_TOKEN_KEY)?.trim() || "";
  } catch {
    return "";
  }
}
function writeAdminToken(token: string) {
  try {
    if (token) sessionStorage.setItem(ADMIN_TOKEN_KEY, token);
    else sessionStorage.removeItem(ADMIN_TOKEN_KEY);
  } catch {
    /* sessionStorage can throw if the browser blocks it */
  }
}
function Login({
  onLogin,
  error: sessionError = "",
}: {
  onLogin: (key: string) => void;
  error?: string;
}) {
  const [key, setKey] = useState(""),
    [error, setError] = useState(""),
    [busy, setBusy] = useState(false);
  const login = async (e: Event) => {
    e.preventDefault();
    setBusy(true);
    setError("");
    try {
      const response = await fetch("/api/overview", {
        headers: { Authorization: `Bearer ${key}` },
      });
      if (!response.ok)
        throw Error(
          response.status === 401
            ? "The admin token is not valid."
            : `Service returned HTTP ${response.status}.`,
        );
      onLogin(key);
      setKey("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Unable to connect");
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
        <span>
          flow<span class="brand-divider">/</span>
          <small>control plane</small>
        </span>
      </div>
      <div class="login-card">
        <span class="login-eyebrow">ONE SERVICE. FULL VISIBILITY.</span>
        <h1>
          Know your system.
          <br />
          Own your operations.
        </h1>
        <p>
          Metrics, streaming, logs and developer tools.
          <br />
          Everything your runtime needs, in one place.
        </p>
        <form onSubmit={login}>
          <label htmlFor="admin-token">Admin access token</label>
          <input
            id="admin-token"
            type="password"
            value={key}
            onInput={(e) => setKey(e.currentTarget.value)}
            placeholder="Enter FLOW_ADMIN_TOKEN"
            autoComplete="off"
            required
            autoFocus
          />
          <ErrorBanner message={error || sessionError} />
          <button class="button-primary w-full" disabled={busy || !key}>
            {busy ? "Connecting…" : "Open control plane"}
            <Icon name="arrow" size={17} />
          </button>
        </form>
        <div class="login-foot">
          <Icon name="audit" size={15} />
          Internal listener · token stays in this tab
        </div>
      </div>
      <p class="login-caption">
        Built into Flow. No agents. No external monitoring stack.
      </p>
    </div>
  );
}
function App() {
  const [token, setToken] = useState(readAdminToken),
    [authError, setAuthError] = useState(""),
    [page, setPage] = useState(currentPage),
    [minutes, setMinutes] = useState(60),
    [interval, setIntervalMs] = useState(5000),
    [mobile, setMobile] = useState(false),
    [authorization, setAuthorization] = useState(""),
    [consoleEndpoint, setConsoleEndpoint] = useState(""),
    [logEndpoint, setLogEndpoint] = useState(""),
    [toast, setToast] = useState("");
  const rememberToken = useCallback((value: string) => {
    writeAdminToken(value);
    setToken(value);
    setAuthError("");
  }, []);
  const [project, setProject] = useState("");
  const api = useCallback<Api>(
    async <T,>(path: string, options: RequestInit = {}) => {
      const projectPath =
        project && path !== "/api/projects"
          ? `${path}${path.includes("?") ? "&" : "?"}project=${encodeURIComponent(project)}`
          : path;
      const response = await fetch(projectPath, {
        ...options,
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Type": "application/json",
          ...options.headers,
        },
      });
      if (response.status === 401) {
        rememberToken("");
        setAuthError("The admin token is not valid.");
        throw Error("The admin token is not valid.");
      }
      if (!response.ok) {
        let message = `HTTP ${response.status}`;
        try {
          const body = await response.json();
          message += `: ${body.message ?? body.error ?? "request failed"}`;
        } catch {}
        throw Error(message);
      }
      return response.json() as Promise<T>;
    },
    [token, rememberToken, project],
  );
  useEffect(() => {
    const handler = () => {
      setPage(currentPage());
      setMobile(false);
    };
    window.addEventListener("hashchange", handler);
    return () => window.removeEventListener("hashchange", handler);
  }, []);
  useEffect(() => {
    if (!toast) return;
    const timer = setTimeout(() => setToast(""), 3000);
    return () => clearTimeout(timer);
  }, [toast]);
  const navigate = (id: string) => {
    location.hash = id;
    setPage(id);
    setMobile(false);
  };
  const copy = async (text: string) => {
    try {
      await navigator.clipboard.writeText(text);
      setToast("Copied to clipboard");
    } catch {
      setToast("Clipboard unavailable. Select the text to copy it.");
    }
  };
  if (!token) return <Login onLogin={rememberToken} error={authError} />;
  return (
    <Authenticated
      key={token}
      token={token}
      project={project}
      initializeProject={(name) => setProject((current) => current || name)}
      setProject={(name) => {
        setProject(name);
        setAuthorization("");
        setConsoleEndpoint("");
        setLogEndpoint("");
      }}
      api={api}
      page={page}
      minutes={minutes}
      setMinutes={setMinutes}
      interval={interval}
      setIntervalMs={setIntervalMs}
      mobile={mobile}
      setMobile={setMobile}
      navigate={navigate}
      authorization={authorization}
      setAuthorization={setAuthorization}
      consoleEndpoint={consoleEndpoint}
      setConsoleEndpoint={setConsoleEndpoint}
      logEndpoint={logEndpoint}
      setLogEndpoint={setLogEndpoint}
      copy={copy}
      toast={toast}
      logout={() => {
        rememberToken("");
        setAuthorization("");
        setConsoleEndpoint("");
        setLogEndpoint("");
      }}
    />
  );
}
type AuthenticatedProps = {
  project: string;
  initializeProject: (name: string) => void;
  setProject: (name: string) => void;
  token: string;
  api: Api;
  page: string;
  minutes: number;
  setMinutes: (n: number) => void;
  interval: number;
  setIntervalMs: (n: number) => void;
  mobile: boolean;
  setMobile: (v: boolean) => void;
  navigate: (id: string) => void;
  authorization: string;
  setAuthorization: (v: string) => void;
  consoleEndpoint: string;
  setConsoleEndpoint: (v: string) => void;
  logEndpoint: string;
  setLogEndpoint: (v: string) => void;
  copy: (v: string) => void;
  toast: string;
  logout: () => void;
};
function Authenticated(props: AuthenticatedProps) {
  const {
    token,
    api,
    page,
    minutes,
    setMinutes,
    interval,
    setIntervalMs,
    mobile,
    setMobile,
    navigate,
    authorization,
    setAuthorization,
    consoleEndpoint,
    setConsoleEndpoint,
    logEndpoint,
    setLogEndpoint,
    copy,
    toast,
    logout,
  } = props;
  const projects = usePolling<{
    projects: {
      name: string;
      active: boolean;
      status: string;
      error?: string;
      generation: number;
    }[];
    default_project: string;
  }>(api, "/api/projects", interval);
  useEffect(() => {
    if (projects.data) props.initializeProject(projects.data.default_project);
  }, [projects.data]);
  const overview = usePolling<Overview>(api, "/api/overview", interval),
    data = overview.data,
    active = pages.find((p) => p.id === page) ?? pages[0];
  const toConsole = (name: string) => {
    if (props.project === "all" && name.includes("/")) {
      const [project, ...rest] = name.split("/");
      props.setProject(project);
      name = rest.join("/");
    }
    setConsoleEndpoint(name);
    navigate("console");
  };
  const toLogs = (endpoint = "") => {
    setLogEndpoint(endpoint);
    navigate("logs");
  };
  const pageProps = data
    ? { data, minutes, onConsole: toConsole, onLogs: toLogs }
    : null;
  return (
    <div class="app-shell">
      {mobile && (
        <button
          class="mobile-backdrop"
          aria-label="Close navigation"
          onClick={() => setMobile(false)}
        />
      )}
      <aside class={`sidebar ${mobile ? "sidebar-open" : ""}`}>
        <a class="brand" href="#overview">
          <span class="brand-mark">
            f<span>.</span>
          </span>
          <span>
            <strong>flow</strong>
            <small>CONTROL PLANE</small>
          </span>
        </a>
        <div class="service-pill">
          <i class="status-dot" />
          <div>
            <strong>Application runtime</strong>
            <small>
              {props.project === "all"
                ? "All projects"
                : props.project || "Connecting…"}
            </small>
          </div>
        </div>
        <select
          class="project-select"
          aria-label="Active project"
          value={props.project}
          disabled={!props.project || !projects.data}
          onChange={(e) => props.setProject(e.currentTarget.value)}
        >
          <option value="all">Entire system</option>
          {projects.data?.projects.map((p) => (
            <option key={p.name} value={p.name} disabled={!p.active}>
              {p.name} · {p.status}
            </option>
          ))}
        </select>
        <nav aria-label="Main navigation">
          {pages.map((p) => (
            <div key={p.id}>
              {p.group && <div class="nav-group">{p.group}</div>}
              <a
                href={`#${p.id}`}
                aria-label={p.title}
                class={`nav-item ${page === p.id ? "active" : ""}`}
                onClick={() => setMobile(false)}
              >
                <Icon name={p.icon} />
                <span>{p.title}</span>
                {p.id === "streaming" && data && (
                  <small>
                    {(data.metrics.service.gauges.mqtt_connections ?? 0) +
                      (data.metrics.service.gauges.ws_connections ?? 0)}
                  </small>
                )}
              </a>
            </div>
          ))}
        </nav>
        <div class="sidebar-footer">
          <div class="version">
            <span class="status-dot" />
            Flow runtime <small>v{data?.version ?? "…"}</small>
          </div>
          <button onClick={logout}>
            <Icon name="logout" size={16} />
            Disconnect admin
          </button>
        </div>
      </aside>
      <div class="main-shell">
        <header class="topbar">
          <div class="flex gap-3 items-center">
            <button
              class="icon-button mobile-menu"
              onClick={() => setMobile(!mobile)}
              aria-label="Open navigation"
            >
              <Icon name="menu" />
            </button>
            <span class="breadcrumb">Control plane</span>
            <span class="breadcrumb-separator">/</span>
            <strong>{active.title}</strong>
          </div>
          <div class="topbar-status">
            <i class={`status-dot ${overview.error ? "status-error" : ""}`} />
            {overview.error ? "Connection issue" : interval ? "Live" : "Paused"}
            <span class="divider" />
            <span class="mono">{location.host}</span>
          </div>
        </header>
        <main class="content">
          <div class="page-header">
            <div>
              <div class="eyebrow">FLOW OPERATIONS</div>
              <h1>{active.title}</h1>
              <p>{active.description}</p>
            </div>
            <div class="header-controls">
              {["overview", "traffic", "streaming", "resources"].includes(
                page,
              ) && (
                <select
                  value={minutes}
                  onChange={(e) => setMinutes(Number(e.currentTarget.value))}
                  aria-label="Chart time range"
                >
                  <option value="15">Last 15 minutes</option>
                  <option value="60">Last hour</option>
                  <option value="360">Last 6 hours</option>
                </select>
              )}
              <select
                value={interval}
                onChange={(e) => setIntervalMs(Number(e.currentTarget.value))}
                aria-label="Auto refresh interval"
              >
                <option value="5000">Refresh · 5s</option>
                <option value="15000">Refresh · 15s</option>
                <option value="30000">Refresh · 30s</option>
                <option value="0">Paused</option>
              </select>
              <button
                class="icon-button refresh-button"
                onClick={() => {
                  overview.refresh();
                  projects.refresh();
                }}
                aria-label="Refresh dashboard"
              >
                <Icon name="refresh" />
              </button>
            </div>
          </div>
          <ErrorBanner message={overview.error} />
          {data && pageProps ? (
            <div class="page-content" key={props.project}>
              {projects.data?.projects
                .filter((p) => p.status !== "running")
                .map((p) => (
                  <ErrorBanner
                    key={p.name}
                    message={`${p.name}: ${p.error ?? p.status} · ${p.active ? "Last working version remains active" : "Project unavailable"}`}
                  />
                ))}
              {(page === "console" || page === "tokens") &&
                props.project === "all" && (
                  <Empty
                    icon="key"
                    title="Select a project"
                    text="Credentials and endpoint calls belong to a specific project."
                  />
                )}
              {page === "database" &&
                (props.project === "all" ? (
                  <Empty
                    icon="resources"
                    title="Select a project"
                    text="Choose a project to browse its isolated database."
                  />
                ) : (
                  <DatabasePage api={api} />
                ))}
              {page === "overview" && <OverviewPage {...pageProps} />}{" "}
              {page === "traffic" && <TrafficPage {...pageProps} />}{" "}
              {page === "streaming" && <StreamingPage {...pageProps} />}{" "}
              {page === "resources" && <ResourcesPage {...pageProps} />}{" "}
              {page === "logs" && (
                <LogsPage
                  key={logEndpoint}
                  api={api}
                  data={data}
                  initialEndpoint={logEndpoint}
                  interval={interval}
                  onCopy={copy}
                />
              )}{" "}
              {page === "audit" && (
                <AuditPage api={api} data={data} onCopy={copy} />
              )}{" "}
              {page === "console" && props.project !== "all" && (
                <ConsolePage
                  key={consoleEndpoint}
                  data={data}
                  adminToken={token}
                  project={props.project}
                  initialEndpoint={consoleEndpoint}
                  authorization={authorization}
                  setAuthorization={setAuthorization}
                  onJwt={() => navigate("tokens")}
                  onCopy={copy}
                />
              )}{" "}
              {page === "tokens" && props.project !== "all" && (
                <TokensPage
                  api={api}
                  onCopy={copy}
                  onUse={(credential) => {
                    setAuthorization(credential);
                    navigate("console");
                  }}
                />
              )}
            </div>
          ) : (
            <Empty
              title="Connecting to the runtime…"
              text="Loading live metrics and application metadata."
            />
          )}
          <footer class="content-footer">
            <span>Flow · Built-in operations</span>
            <span>
              Bounded telemetry · Minute buckets · Approximate percentiles
            </span>
          </footer>
        </main>
      </div>
      {toast && (
        <div class="toast" role="status">
          <Icon name="copy" size={16} />
          {toast}
        </div>
      )}
    </div>
  );
}
render(<App />, document.getElementById("app")!);
