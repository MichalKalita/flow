export type HistoryPoint = {
  minute: number;
  count: number;
  errors: number;
  mean_ms: number;
  p50_ms?: number | null;
  p95_ms?: number | string | null;
  p99_ms?: number | string | null;
  buckets?: number[];
};
export type Metric = {
  count: number;
  errors: number;
  mean_ms: number;
  sum_ms: number;
  p50_ms: number | null;
  p95_ms: number | string | null;
  p99_ms: number | string | null;
  buckets: number[];
  statuses: number[];
  history: HistoryPoint[];
};
export type Endpoint = {
  name: string;
  method: string;
  path: string;
  mutation: boolean;
  status: number;
  inputs: { name: string; type: string; default?: unknown }[];
};
export type ProcessMemory = {
  rss_bytes?: number;
  peak_rss_bytes?: number;
  cpu_percent?: number;
  cpu_seconds?: number;
  source?: string;
};
export type HostMemory = {
  limit_bytes?: number | null;
  physical_bytes?: number | null;
  available_bytes?: number | null;
  source?: string;
};
export type ServicePoint = {
  minute: number;
  counts: Record<string, number>;
  gauges: Record<string, number>;
  resources: ProcessMemory;
};
export type Overview = {
  version: string;
  scope?: string;
  endpoints: Endpoint[];
  streams: {
    name: string;
    topic: string;
    retention_seconds: number;
    max_messages: number;
  }[];
  automations: { name: string; source: string; actor: string }[];
  metrics: {
    endpoints: Record<string, Metric>;
    system?: Metric;
    work: Record<string, Metric>;
    histogram_bounds_ms: number[];
    dropped_logs: number;
    storage_errors: number;
    service: {
      counters: Record<string, number>;
      gauges: Record<string, number>;
      uptime_seconds: number;
      resources: ProcessMemory;
      history: ServicePoint[];
      seconds: { second: number; counts: Record<string, number> }[];
    };
  };
  resources: {
    process: ProcessMemory;
    host?: HostMemory;
    sqlite: Record<string, number | boolean | null>;
    observability: Record<string, number | string | null>;
  };
};
export type Log = {
  sequence: number;
  time: string;
  kind: string;
  level: string;
  endpoint?: string;
  name?: string;
  duration_ms?: number;
  status?: number;
  request_id?: string;
  error?: string;
  [key: string]: unknown;
};
export type LogPage = {
  entries: Log[];
  next_cursor: string | null;
  scan_limited: boolean;
  cursor_expired: boolean;
  scanned_bytes: number;
  source: string;
};
export type Audit = {
  id: number;
  time: string;
  operation: string;
  transport: string;
  actor: unknown;
  entity: string;
  entity_id: number;
  action: string;
  transaction_id: string;
  cursor?: string;
  project?: string;
  before: unknown;
  after: unknown;
};
export type Adapter = {
  alias: string;
  issuer: string;
  audience: string;
  configured: boolean;
  subjects: { subject: string; actor_id: number }[];
};
export type IssuedToken = {
  token: string;
  authorization: string;
  claims: { exp: number; sub: string; [key: string]: unknown };
};
export type Api = <T>(path: string, options?: RequestInit) => Promise<T>;
