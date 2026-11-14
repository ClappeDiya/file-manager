/**
 * Local volume cache (K77): usage and "Clear cache" (Standard); folder,
 * size limit and use for full restores (Advanced).
 */
import { useCallback, useEffect, useState } from "react";
import { errorText } from "@/lib/error-text";
import { formatBytes } from "@/lib/format-bytes";
import { inputCls } from "@/lib/backup-helpers";
import { cacheStats, clearCache, DEFAULT_CACHE_SETTINGS, getCacheSettings, setCacheSettings, type CacheSettings, type CacheStats } from "@/stores/backup-access";
import { AdvancedOnly } from "./advanced-only";
import { Banner, Btn, Check, Field, PathInput, Section } from "./backup-ui";

const GIB = 1024 ** 3;

export function BackupCacheSettings() {
  const [stats, setStats] = useState<CacheStats | null>(null);
  const [settings, setSettings] = useState<CacheSettings>(DEFAULT_CACHE_SETTINGS);
  const [note, setNote] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const [s, c] = await Promise.all([cacheStats(), getCacheSettings()]);
      // Older backends (or test doubles) may not know these commands.
      setStats(s ?? null);
      setSettings(c ?? DEFAULT_CACHE_SETTINGS);
    } catch (e) {
      setError(errorText(e));
    }
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (fn: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await fn();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const off = settings.max_bytes === 0;
  return (
    <Section title="Volume cache" hint="Previews, drives and repeated reads keep downloaded backup data here (still encrypted), so it is not downloaded again.">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {note && <Banner tone="success" onClose={() => setNote(null)}>{note}</Banner>}
      <p className="text-xs text-zinc-600 dark:text-zinc-400" data-testid="backup-cache-usage">
        {off
          ? "The cache is off."
          : stats
            ? `${formatBytes(stats.bytes)} of ${formatBytes(stats.max_bytes)} used · ${stats.entries} volume(s)${stats.hits + stats.misses > 0 ? ` · ${Math.round(stats.hit_rate * 100)}% served from the cache` : ""}`
            : "…"}
      </p>
      <Btn
        disabled={busy || !stats || stats.entries === 0}
        testId="backup-cache-clear"
        onClick={() =>
          run(async () => {
            const freed = await clearCache();
            setNote(`Freed ${formatBytes(freed)}.`);
            await refresh();
          })
        }
      >
        Clear cache
      </Btn>
      <AdvancedOnly>
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Cache folder (empty = the app's cache folder)">
            <PathInput value={settings.dir ?? ""} onChange={(d) => setSettings({ ...settings, dir: d.trim() || null })} placeholder={stats?.dir || "default"} testId="backup-cache-dir" />
          </Field>
          <Field label="Size limit (GiB, 0 = off)">
            <input
              className={inputCls}
              type="number"
              min={0}
              step={0.5}
              value={Math.round((settings.max_bytes / GIB) * 100) / 100}
              onChange={(e) => setSettings({ ...settings, max_bytes: Math.max(0, Math.round(Number(e.target.value) * GIB)) })}
              aria-label="Cache size limit"
              data-testid="backup-cache-size"
            />
          </Field>
        </div>
        <Check label="Also use the cache for full restores" checked={settings.use_for_restore} onChange={(v) => setSettings({ ...settings, use_for_restore: v })} hint="Off: a large restore reads each volume once and would only push everything else out." />
        <Btn
          tone="primary"
          disabled={busy}
          testId="backup-cache-save"
          onClick={() =>
            run(async () => {
              setStats((await setCacheSettings(settings)) ?? null);
              setNote("Cache settings saved.");
            })
          }
        >
          Save cache settings
        </Btn>
      </AdvancedOnly>
    </Section>
  );
}
