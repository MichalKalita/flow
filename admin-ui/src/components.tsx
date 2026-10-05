import type { ComponentChildren } from "preact";
import { useId, useState } from "preact/hooks";
import { colors, formatNumber, type PlotPoint } from "./data";
const icons: Record<string, string> = {
  overview: "M3 3h7v7H3z M14 3h7v7h-7z M3 14h7v7H3z M14 14h7v7h-7z",
  traffic: "M3 17l5-8 4 5 5-11 4 6 M3 21h18",
  stream: "M4 5h5v5H4z M15 14h5v5h-5z M6.5 10v6h8.5 M15 5h5 M17.5 2.5v5",
  resources:
    "M7 7h10v10H7z M9 1v3 M15 1v3 M9 20v3 M15 20v3 M1 9h3 M1 15h3 M20 9h3 M20 15h3",
  logs: "M5 3h14v18H5z M8 7h8 M8 11h8 M8 15h5",
  audit: "M12 3l8 3v6c0 5-8 9-8 9s-8-4-8-9V6z M8 12l3 3 5-6",
  console: "M4 6l6 6-6 6 M13 18h7",
  key: "M14 14a6 6 0 1 0-4-4 M10 14l-7 7 M5 19l-2-2 M8 16l-2-2",
  arrow: "M5 12h14 M13 6l6 6-6 6",
  refresh:
    "M20 7v5h-5 M4 17v-5h5 M6 7a7 7 0 0 1 12-2l2 2 M18 17a7 7 0 0 1-12 2l-2-2",
  search: "M10 17a7 7 0 1 0 0-14 7 7 0 0 0 0 14 M15 15l6 6",
  chevron: "M9 5l7 7-7 7",
  close: "M6 6l12 12 M6 18L18 6",
  copy: "M9 9h12v12H9z M15 9V3H3v12h6",
  pause: "M8 5v14 M16 5v14",
  play: "M6 4l14 8-14 8z",
  logout: "M9 4H4v16h5 M10 12h11 M17 8l4 4-4 4",
  menu: "M3 6h18 M3 12h18 M3 18h18",
  info: "M12 17v-6 M12 7v.1 M12 22a10 10 0 1 0 0-20 10 10 0 0 0 0 20",
};
export function Icon({ name, size = 18 }: { name: string; size?: number }) {
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      stroke-width="1.6"
      stroke-linecap="round"
      stroke-linejoin="round"
      aria-hidden="true"
    >
      <path d={icons[name] ?? icons.info} />
    </svg>
  );
}
export function Badge({
  children,
  tone = "neutral",
}: {
  children: ComponentChildren;
  tone?: string;
}) {
  return <span class={`badge badge-${tone}`}>{children}</span>;
}
export function Panel({
  title,
  description,
  action,
  children,
  className = "",
}: {
  title: string;
  description?: string;
  action?: ComponentChildren;
  children: ComponentChildren;
  className?: string;
}) {
  return (
    <section class={`panel ${className}`}>
      <header class="panel-heading">
        <div>
          <h2>{title}</h2>
          {description && <p>{description}</p>}
        </div>
        {action}
      </header>
      {children}
    </section>
  );
}
export function Stat({
  label,
  value,
  detail,
  icon,
  tone = "",
}: {
  label: string;
  value: ComponentChildren;
  detail: ComponentChildren;
  icon: string;
  tone?: string;
}) {
  return (
    <div class="stat">
      <div class="flex items-center justify-between">
        <span class="stat-label">{label}</span>
        <span class="stat-icon">
          <Icon name={icon} />
        </span>
      </div>
      <div class={`stat-value ${tone}`}>{value}</div>
      <div class="stat-detail">{detail}</div>
    </div>
  );
}
export function Empty({
  icon = "traffic",
  title = "No activity yet",
  text = "Data will appear as your application handles traffic.",
}: {
  icon?: Parameters<typeof Icon>[0]["name"];
  title?: string;
  text?: string;
}) {
  return (
    <div class="empty">
      <span class="empty-icon">
        <Icon name={icon} size={25} />
      </span>
      <strong>{title}</strong>
      <p>{text}</p>
    </div>
  );
}
export function ErrorBanner({ message }: { message?: string }) {
  return message ? (
    <div class="error-banner" role="alert">
      <Icon name="info" />
      {message}
    </div>
  ) : null;
}
export function Drawer({
  title,
  onClose,
  children,
}: {
  title: string;
  onClose: () => void;
  children: ComponentChildren;
}) {
  return (
    <div class="drawer-backdrop" onClick={onClose}>
      <section
        class="drawer"
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onClick={(e) => e.stopPropagation()}
      >
        <header>
          <h2>{title}</h2>
          <button
            class="icon-button"
            onClick={onClose}
            aria-label="Close details"
          >
            <Icon name="close" />
          </button>
        </header>
        {children}
      </section>
    </div>
  );
}
export function Chart({
  data,
  series,
  unit = "",
  formatter,
  title,
}: {
  data: PlotPoint[];
  series: { key: string; label: string; color: string }[];
  unit?: string;
  formatter?: (n: number) => string;
  title: string;
}) {
  const [hover, setHover] = useState<number | null>(null),
    id = useId().replace(/:/g, "");
  const display = formatter ?? ((n: number) => `${formatNumber(n, 1)}${unit}`);
  const max =
    Math.max(
      1,
      ...data.flatMap((p) =>
        series.map((s) => {
          const n = p[s.key];
          return typeof n === "number" && Number.isFinite(n) ? n : 0;
        }),
      ),
    ) * 1.15;
  const x = (i: number) =>
      52 + (data.length <= 1 ? 0 : i / (data.length - 1)) * 808,
    y = (v: number) => 205 - (v / max) * 170;
  const paths = (key: string) => {
    const chunks: { x: number; y: number }[][] = [];
    let current: { x: number; y: number }[] = [];
    data.forEach((p, i) => {
      const v = p[key];
      if (v == null || !Number.isFinite(v)) {
        if (current.length) chunks.push(current);
        current = [];
      } else current.push({ x: x(i), y: y(v) });
    });
    if (current.length) chunks.push(current);
    return chunks;
  };
  const time = (n: number) =>
    new Date(n).toLocaleTimeString("en-GB", {
      hour: "2-digit",
      minute: "2-digit",
    });
  return (
    <div class="chart">
      <div class="chart-legend">
        {series.map((s) => (
          <span key={s.key}>
            <i style={{ background: s.color }} />
            {s.label}
          </span>
        ))}
        <small>{unit}</small>
      </div>
      <div class="chart-canvas" onMouseLeave={() => setHover(null)}>
        <svg
          viewBox="0 0 900 240"
          role="img"
          aria-label={title}
          onMouseMove={(e) => {
            const rect = e.currentTarget.getBoundingClientRect();
            setHover(
              Math.round(
                Math.max(
                  0,
                  Math.min(
                    1,
                    (((e.clientX - rect.left) / rect.width) * 900 - 52) / 808,
                  ),
                ) *
                  (data.length - 1),
              ),
            );
          }}
        >
          <defs>
            {series.map((s) => (
              <linearGradient id={`${id}-${s.key}`} x1="0" x2="0" y1="0" y2="1">
                <stop offset="0%" stop-color={s.color} stop-opacity=".13" />
                <stop offset="100%" stop-color={s.color} stop-opacity="0" />
              </linearGradient>
            ))}
          </defs>
          {[0, 1, 2, 3, 4].map((i) => (
            <g key={i}>
              <line
                x1="52"
                x2="860"
                y1={y((max * i) / 4)}
                y2={y((max * i) / 4)}
                stroke="#e9edf0"
                stroke-dasharray={i ? "3 5" : "0"}
              />
              <text
                x="42"
                y={y((max * i) / 4) + 4}
                text-anchor="end"
                fill="#88949f"
                font-size="12"
              >
                {formatter
                  ? display((max * i) / 4)
                  : formatNumber((max * i) / 4, 1)}
              </text>
            </g>
          ))}
          {series.map((s, si) =>
            paths(s.key).map((path, i) => (
              <g key={`${s.key}-${i}`}>
                {si === 0 && path.length > 1 && (
                  <path
                    d={`M${path[0].x},205 ${path.map((p) => `L${p.x},${p.y}`).join(" ")} L${path.at(-1)!.x},205 Z`}
                    fill={`url(#${id}-${s.key})`}
                  />
                )}
                <polyline
                  points={path.map((p) => `${p.x},${p.y}`).join(" ")}
                  fill="none"
                  stroke={s.color}
                  stroke-width="2"
                  stroke-linejoin="round"
                />
                {path.length === 1 && (
                  <circle cx={path[0].x} cy={path[0].y} r="3" fill={s.color} />
                )}
              </g>
            )),
          )}
          {[0, 0.25, 0.5, 0.75, 1].map((f) => {
            const i = Math.round((data.length - 1) * f);
            return (
              data[i] && (
                <text
                  x={x(i)}
                  y="228"
                  text-anchor="middle"
                  fill="#88949f"
                  font-size="12"
                  key={f}
                >
                  {time(data[i].time)}
                </text>
              )
            );
          })}
          {hover != null && data[hover] && (
            <g>
              <line
                x1={x(hover)}
                x2={x(hover)}
                y1="24"
                y2="205"
                stroke="#a6b1ba"
                stroke-dasharray="3 4"
              />
              {series.map((s) => {
                const v = data[hover][s.key];
                return (
                  v != null &&
                  Number.isFinite(v) && (
                    <circle
                      key={s.key}
                      cx={x(hover)}
                      cy={y(v)}
                      r="4"
                      stroke="white"
                      stroke-width="2"
                      fill={s.color}
                    />
                  )
                );
              })}
            </g>
          )}
        </svg>
        {hover != null && data[hover] && (
          <div class="chart-tooltip">
            <strong>{time(data[hover].time)}</strong>
            {series.map((s) => (
              <div key={s.key}>
                <span>
                  <i style={{ background: s.color }} />
                  {s.label}
                </span>
                <b>
                  {data[hover][s.key] == null
                    ? "No sample"
                    : display(data[hover][s.key]!)}
                </b>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
export function ResourceBar({
  label,
  value,
  total,
  color = colors.green,
  detail,
}: {
  label: string;
  value: number;
  total: number;
  color?: string;
  detail: string;
}) {
  return (
    <div class="resource-bar">
      <div class="flex justify-between">
        <span>{label}</span>
        <strong>{detail}</strong>
      </div>
      <div class="bar-track">
        <div
          style={{
            width: `${Math.min(100, (value / Math.max(1, total)) * 100)}%`,
            background: color,
          }}
        />
      </div>
    </div>
  );
}
