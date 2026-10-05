import { expect, test } from "bun:test";
import { percentile, requestSeries, windowStats } from "./data";
import type { Metric } from "./types";
test("aggregate weighted latency and histogram percentiles across endpoints", () => {
  const minute = 12345,
    now = minute * 60000;
  const metrics = {
    fast: {
      history: [
        {
          minute,
          count: 99,
          errors: 0,
          mean_ms: 1,
          buckets: [99, ...Array(15).fill(0)],
        },
      ],
    },
    slow: {
      history: [
        {
          minute,
          count: 1,
          errors: 1,
          mean_ms: 100,
          buckets: [0, 0, 0, 0, 0, 0, 1, ...Array(9).fill(0)],
        },
      ],
    },
  } as unknown as Record<string, Metric>;
  const point = requestSeries(metrics, 1, now)[0];
  expect(point.mean_ms).toBe(1.99);
  expect(point.p95_ms).toBe(1);
  expect(point.count).toBe(100);
  const stats = windowStats(metrics, 1, [1, 2, 5, 10, 20, 50, 100], now);
  expect(stats.errorRate).toBe(1);
  expect(stats.p99).toBe(1);
});
test("empty latency has no fabricated zero sample; overflow is explicit", () => {
  expect(requestSeries({}, 1)[0].mean_ms).toBeNull();
  expect(percentile(Array(16).fill(0), [1], 0.95)).toBeNull();
  expect(
    percentile(
      [...Array(15).fill(0), 1],
      [
        1, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 30000,
        60000,
      ],
      0.95,
    ),
  ).toBe(Infinity);
});
