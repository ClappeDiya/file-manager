'use client';

import React from 'react';
import { Badge } from '@/components/ui/badge';

const CONNECTIVITY: Record<string, { label: string; variant: 'success' | 'warning' | 'error' | 'default' }> = {
  online: { label: 'Online', variant: 'success' },
  stale: { label: 'Not seen recently', variant: 'warning' },
  offline: { label: 'Offline', variant: 'error' },
  never: { label: 'Never connected', variant: 'default' },
};

export function ConnectivityBadge({ value }: { value: string }) {
  const c = CONNECTIVITY[value] ?? CONNECTIVITY.never;
  return <Badge variant={c.variant}>{c.label}</Badge>;
}

const RESULT: Record<string, 'success' | 'warning' | 'error' | 'default' | 'info'> = {
  success: 'success',
  succeeded: 'success',
  warning: 'warning',
  error: 'error',
  failed: 'error',
  cancelled: 'default',
  queued: 'info',
  delivered: 'info',
  expired: 'default',
  active: 'success',
  suspended: 'warning',
  revoked: 'error',
  offboarding: 'warning',
  pending: 'info',
  accepted: 'success',
  dead: 'error',
  paused: 'warning',
  disabled: 'default',
};

export function StatusBadge({ value }: { value: string | null }) {
  if (!value) return <Badge variant="outline">Never run</Badge>;
  return <Badge variant={RESULT[value] ?? 'default'}>{value.replace(/_/g, ' ')}</Badge>;
}

/** A tag with its organization-defined color (falls back to neutral). */
export function TagChip({ tag, color, onRemove, onClick, active }: { tag: string; color?: string | null; onRemove?: () => void; onClick?: () => void; active?: boolean }) {
  const style = color ? { backgroundColor: `${color}22`, borderColor: color, color } : undefined;
  return (
    <span
      className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 text-xs font-medium ${color ? '' : 'border-border text-foreground-secondary'} ${onClick ? 'cursor-pointer' : ''} ${active ? 'ring-2 ring-primary' : ''}`}
      style={style}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
      onKeyDown={onClick ? (e) => (e.key === 'Enter' || e.key === ' ') && onClick() : undefined}
    >
      {tag}
      {onRemove && (
        <button type="button" aria-label={`Remove tag ${tag}`} className="ml-0.5 opacity-70 hover:opacity-100" onClick={(e) => (e.stopPropagation(), onRemove())}>
          ×
        </button>
      )}
    </span>
  );
}

export function ErrorBanner({ message }: { message: string | null }) {
  if (!message) return null;
  return (
    <div role="alert" className="mb-4 rounded-md border border-error bg-error-bg px-4 py-3 text-sm text-error-foreground">
      {message}
    </div>
  );
}

export function Loading() {
  return <div className="py-12 text-center text-sm text-foreground-secondary">Loading…</div>;
}
