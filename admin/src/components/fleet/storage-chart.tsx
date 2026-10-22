'use client';

import React, { useState } from 'react';
import { formatBytes } from '@/lib/utils/format';

export interface StorageDay {
  day: string;
  backendSize: number;
  sourceSize: number;
  runs: number;
  failures: number;
}

/**
 * Daily backend storage as a single-series bar chart (magnitude → one hue),
 * hover tooltip per bar, and a table view for screen readers and exact values.
 */
export function StorageChart({ days }: { days: StorageDay[] }) {
  const [hover, setHover] = useState<number | null>(null);
  const [table, setTable] = useState(false);
  const max = Math.max(1, ...days.map((d) => d.backendSize));
  if (!days.length) return <p className="py-8 text-center text-sm text-foreground-secondary">No reports in this period yet.</p>;
  const h = hover !== null ? days[hover] : null;
  return (
    <div>
      <div className="mb-2 flex items-center justify-between text-xs text-foreground-secondary">
        <span>Max {formatBytes(max)}</span>
        <button type="button" className="underline" onClick={() => setTable((t) => !t)}>
          {table ? 'Show chart' : 'Show table'}
        </button>
      </div>
      {table ? (
        <div className="max-h-64 overflow-auto">
          <table className="w-full text-sm">
            <thead>
              <tr className="text-left text-xs text-foreground-secondary">
                <th className="py-1">Day</th>
                <th>Stored</th>
                <th>Source</th>
                <th>Runs</th>
                <th>Failed</th>
              </tr>
            </thead>
            <tbody>
              {days.map((d) => (
                <tr key={d.day} className="border-t border-border">
                  <td className="py-1">{d.day}</td>
                  <td>{formatBytes(d.backendSize)}</td>
                  <td>{formatBytes(d.sourceSize)}</td>
                  <td>{d.runs}</td>
                  <td>{d.failures}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      ) : (
        <div className="relative">
          <div className="flex h-40 items-end gap-[2px] border-b border-border" role="img" aria-label={`Stored data per day, ${days[0].day} to ${days[days.length - 1].day}`}>
            {days.map((d, i) => (
              <div key={d.day} className="flex h-full flex-1 items-end" onMouseEnter={() => setHover(i)} onMouseLeave={() => setHover(null)}>
                <div className={`w-full rounded-t-[4px] bg-primary ${hover === i ? 'opacity-100' : 'opacity-70'}`} style={{ height: `${Math.max(1, (d.backendSize / max) * 100)}%` }} />
              </div>
            ))}
          </div>
          {h && (
            <div className="pointer-events-none absolute top-0 rounded-md border border-border bg-background-elevated px-3 py-2 text-xs shadow-md" style={{ left: `${Math.min(70, (hover! / days.length) * 100)}%` }}>
              <p className="font-medium text-foreground">{h.day}</p>
              <p className="text-foreground-secondary">Stored {formatBytes(h.backendSize)}</p>
              <p className="text-foreground-secondary">
                {h.runs} run(s){h.failures ? `, ${h.failures} failed` : ''}
              </p>
            </div>
          )}
          <div className="mt-1 flex justify-between text-xs text-foreground-tertiary">
            <span>{days[0].day}</span>
            <span>{days[days.length - 1].day}</span>
          </div>
        </div>
      )}
    </div>
  );
}
