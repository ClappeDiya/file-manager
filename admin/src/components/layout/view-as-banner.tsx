'use client';

/** Shown across the console while an administrator views it as a member (read-only). */
import React from 'react';
import { Eye } from 'lucide-react';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { errorText } from '@/lib/api-client';
import { formatDateTime } from '@/lib/utils/format';

export function ViewAsBanner() {
  const viewAs = useAuthStore((s) => s.viewAs);
  const user = useAuthStore((s) => s.user);
  const stop = useAuthStore((s) => s.stopViewAs);
  const notify = useToastStore((s) => s.notify);
  if (!viewAs || !user) return null;
  return (
    <div role="status" className="flex items-center justify-between gap-3 bg-warning px-4 py-2 text-sm text-white">
      <span className="flex items-center gap-2">
        <Eye size={16} aria-hidden="true" />
        Viewing as <b>{user.email}</b> (read-only) until {formatDateTime(viewAs.until)} · signed in as {viewAs.actor.email}
      </span>
      <button type="button" className="rounded border border-white/70 px-2 py-0.5 hover:bg-white/10" onClick={() => stop().catch((e) => notify(errorText(e), 'error'))}>
        Stop viewing
      </button>
    </div>
  );
}
