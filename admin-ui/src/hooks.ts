import { useCallback, useEffect, useState } from "preact/hooks";
import type { Api } from "./types";
export function usePolling<T>(api: Api, path: string, interval = 0) {
  const [data, setData] = useState<T | null>(null),
    [error, setError] = useState(""),
    [loading, setLoading] = useState(true),
    [revision, setRevision] = useState(0);
  const refresh = useCallback(() => setRevision((v) => v + 1), []);
  useEffect(() => {
    setData(null);
    setError("");
  }, [api, path]);
  useEffect(() => {
    const abort = new AbortController();
    let running = false;
    const load = async () => {
      if (running) return;
      running = true;
      try {
        const value = await api<T>(path, { signal: abort.signal });
        if (!abort.signal.aborted) {
          setData(value);
          setError("");
        }
      } catch (e) {
        if (!abort.signal.aborted)
          setError(e instanceof Error ? e.message : "Request failed");
      } finally {
        if (!abort.signal.aborted) setLoading(false);
        running = false;
      }
    };
    setLoading(true);
    void load();
    const timer = interval
      ? setInterval(() => {
          if (!document.hidden) void load();
        }, interval)
      : undefined;
    return () => {
      abort.abort();
      clearInterval(timer);
    };
  }, [api, path, interval, revision]);
  return { data, error, loading, refresh };
}
export function useDebounce(value: string, delay = 300) {
  const [debounced, setDebounced] = useState(value);
  useEffect(() => {
    const timer = setTimeout(() => setDebounced(value), delay);
    return () => clearTimeout(timer);
  }, [value, delay]);
  return debounced;
}
