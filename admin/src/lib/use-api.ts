'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import { api, errorText } from './api-client';

/** Fetch a GET endpoint; `reload()` refetches, `refreshMs` polls while the page is visible. */
export function useApi<T>(path: string | null, refreshMs?: number) {
  const [data, setData] = useState<T | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(!!path);
  const current = useRef(path);
  current.current = path;

  const reload = useCallback(async () => {
    if (!path) return;
    try {
      const d = await api<T>(path);
      if (current.current === path) {
        setData(d);
        setError(null);
      }
    } catch (e) {
      if (current.current === path) setError(errorText(e));
    } finally {
      if (current.current === path) setLoading(false);
    }
  }, [path]);

  useEffect(() => {
    setLoading(!!path);
    void reload();
    if (!refreshMs) return;
    const t = setInterval(() => {
      if (document.visibilityState === 'visible') void reload();
    }, refreshMs);
    return () => clearInterval(t);
  }, [reload, refreshMs, path]);

  return { data, error, loading, reload, setData };
}
