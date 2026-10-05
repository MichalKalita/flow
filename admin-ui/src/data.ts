import type { HistoryPoint, Metric, Overview, ServicePoint } from "./types";
export type PlotPoint = { time: number; [key: string]: number | null };
export const colors = {
  green: "#159780",
  orange: "#ed8a3b",
  blue: "#6486d5",
  red: "#d55d70",
  purple: "#a183cd",
};
export function formatNumber(n: number | null | undefined, digits = 0): string {
  return n == null || !Number.isFinite(n)
    ? "—"
    : new Intl.NumberFormat("en", { maximumFractionDigits: digits }).format(n);
}
export function bytes(n: unknown): string {
  if (typeof n !== "number") return "—";
  const units = ["B", "KiB", "MiB", "GiB"];
  let i = 0;
  while (n >= 1024 && i < units.length - 1) {
    n /= 1024;
    i++;
  }
  return `${formatNumber(n, i === 0 ? 0 : 1)} ${units[i]}`;
}
export function duration(seconds: number): string {
  if (seconds < 60) return `${Math.floor(seconds)}s`;
  if (seconds < 3600)
    return `${Math.floor(seconds / 60)}m ${Math.floor(seconds % 60)}s`;
  return `${Math.floor(seconds / 3600)}h ${Math.floor((seconds % 3600) / 60)}m`;
}
export function percentile(
  buckets: number[],
  bounds: number[],
  fraction: number,
): number | null {
  const count = buckets.reduce((a, b) => a + b, 0);
  if (!count) return null;
  let sum = 0;
  for (let i = 0; i < buckets.length; i++) {
    sum += buckets[i];
    if (sum >= Math.ceil(count * fraction)) return bounds[i] ?? Infinity;
  }
  return null;
}
export function requestSeries(
  metrics: Record<string, Metric>,
  minutes: number,
  now = Date.now(),
): PlotPoint[] {
  const end = Math.floor(now / 60000);
  const byMinute = new Map<number, HistoryPoint & { buckets: number[] }>();
  for (const metric of Object.values(metrics))
    for (const point of metric.history) {
      const previous = byMinute.get(point.minute) ?? {
        minute: point.minute,
        count: 0,
        errors: 0,
        mean_ms: 0,
        buckets: Array(16).fill(0),
      };
      previous.mean_ms += point.mean_ms * point.count;
      previous.count += point.count;
      previous.errors += point.errors;
      point.buckets?.forEach((n, i) => (previous.buckets[i] += n));
      byMinute.set(point.minute, previous);
    }
  return Array.from({ length: minutes }, (_, i) => {
    const minute = end - minutes + 1 + i,
      p = byMinute.get(minute);
    return {
      time: minute * 60000,
      count: p?.count ?? 0,
      errors: p?.errors ?? 0,
      mean_ms: p?.count ? p.mean_ms / p.count : null,
      p95_ms: p
        ? percentile(
            p.buckets,
            [
              1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000,
              30000, 60000,
            ],
            0.95,
          )
        : null,
      p99_ms: p
        ? percentile(
            p.buckets,
            [
              1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000,
              30000, 60000,
            ],
            0.99,
          )
        : null,
    };
  });
}
export function serviceSeries(
  history: ServicePoint[],
  minutes: number,
  fields: { key: string; category: "counts" | "gauges" | "resources" }[],
  now = Date.now(),
): PlotPoint[] {
  const end = Math.floor(now / 60000),
    map = new Map(history.map((p) => [p.minute, p]));
  return Array.from({ length: minutes }, (_, i) => {
    const minute = end - minutes + 1 + i,
      p = map.get(minute);
    return Object.fromEntries([
      ["time", minute * 60000],
      ...fields.map((f) => [
        f.key,
        p
          ? ((p[f.category] as Record<string, number>)?.[f.key] ??
            (f.category === "counts" ? 0 : null))
          : f.category === "counts"
            ? 0
            : null,
      ]),
    ]) as PlotPoint;
  });
}
export function windowStats(
  metrics: Record<string, Metric>,
  minutes: number,
  bounds: number[],
  now = Date.now(),
) {
  const cutoff = Math.floor(now / 60000) - minutes + 1;
  const buckets = Array(16).fill(0) as number[];
  let count = 0,
    errors = 0,
    sum = 0;
  for (const m of Object.values(metrics))
    for (const p of m.history)
      if (p.minute >= cutoff) {
        count += p.count;
        errors += p.errors;
        sum += p.count * p.mean_ms;
        p.buckets?.forEach((n, i) => (buckets[i] += n));
      }
  return {
    count,
    errors,
    errorRate: count ? (errors / count) * 100 : 0,
    mean: count ? sum / count : 0,
    p95: percentile(buckets, bounds, 0.95),
    p99: percentile(buckets, bounds, 0.99),
  };
}
export function rate(data: Overview, key: string, now = Date.now()): number {
  return (
    data.metrics.service.seconds
      .filter((p) => p.second > now / 1000 - 60)
      .reduce((sum, p) => sum + (p.counts[key] ?? 0), 0) / 60
  );
}
export function latency(n: number | string | null | undefined): string {
  if (n == null) return "—";
  if (n === Infinity || typeof n === "string") return ">60 s";
  return n >= 1000
    ? `${formatNumber(n / 1000, 2)} s`
    : `${formatNumber(n, 2)} ms`;
}
