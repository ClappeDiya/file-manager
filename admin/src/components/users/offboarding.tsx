'use client';

/**
 * Users → removing a member (with a successor), setting when a membership
 * ends, and restoring recently removed members (`lib/offboarding`).
 */
import React, { useState } from 'react';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { Card } from '@/components/ui/card';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import { formatDateTime, formatRelativeTime, getDisplayTimeZone } from '@/lib/utils/format';
import { zonedToUtc } from '@/lib/zoned-time';
import { ROLE_LABELS, type AdminUser } from '@/lib/types/auth';
import type { RemovedMember } from '@/lib/offboarding';

/** A datetime-local value, read in the time zone the console shows times in. */
const fromLocalInput = (v: string) => {
  const [y, m, d, hh, mm] = v.split(/[-T:]/).map(Number);
  return zonedToUtc(y, m, d, hh, mm, getDisplayTimeZone()).toISOString();
};

const patch = (body: Record<string, unknown>) => api('/api/users', { method: 'PATCH', body });

export function RemoveMemberDialog({ user, members, restoreDays, onClose, onDone }: { user: AdminUser | null; members: AdminUser[]; restoreDays: number; onClose: () => void; onDone: () => void }) {
  const [successor, setSuccessor] = useState('');
  const [err, setErr] = useState<string | null>(null);
  if (!user) return null;
  const remove = async () => {
    setErr(null);
    try {
      await patch({ action: 'remove', userIds: [user.id], successorId: successor || null });
      setSuccessor('');
      onDone();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Dialog open onClose={onClose}>
      <DialogHeader>
        <DialogTitle>Remove {user.name}</DialogTitle>
        <DialogDescription>
          {user.email} leaves this organization and its groups; their API tokens here are revoked. If the account belongs to no other organization it is disabled. You can restore them for {restoreDays} days.
        </DialogDescription>
      </DialogHeader>
      <ErrorBanner message={err} />
      <Select
        label="Hand over to (optional)"
        options={[{ value: '', label: 'Nobody' }, ...members.filter((m) => m.id !== user.id && m.isActive).map((m) => ({ value: m.id, label: `${m.name} (${m.email})` }))]}
        value={successor}
        onChange={(e) => setSuccessor(e.target.value)}
      />
      <p className="mt-1 text-xs text-foreground-tertiary">The successor becomes owner of groups this person was the last owner of, and replaces them as a recipient of alert emails.</p>
      <DialogFooter>
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="danger" onClick={() => void remove()}>
          Remove
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

export function MembershipExpiryDialog({ user, onClose, onDone }: { user: (AdminUser & { expiresAt?: string | null }) | null; onClose: () => void; onDone: () => void }) {
  const [date, setDate] = useState('');
  const [err, setErr] = useState<string | null>(null);
  if (!user) return null;
  const save = async (expiresAt: string | null) => {
    setErr(null);
    try {
      await patch({ action: 'set_expiry', userIds: [user.id], expiresAt });
      setDate('');
      onDone();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Dialog open onClose={onClose}>
      <DialogHeader>
        <DialogTitle>Membership end for {user.name}</DialogTitle>
        <DialogDescription>
          {user.expiresAt ? `Currently ends ${formatDateTime(user.expiresAt)}.` : 'No end date.'} On that date the member is removed (and can be restored for a while), as if removed by hand.
        </DialogDescription>
      </DialogHeader>
      <ErrorBanner message={err} />
      <Input label={`Ends on (${getDisplayTimeZone()})`} type="datetime-local" value={date} onChange={(e) => setDate(e.target.value)} />
      <DialogFooter>
        {user.expiresAt && (
          <Button variant="ghost" onClick={() => void save(null)}>
            Remove end date
          </Button>
        )}
        <Button variant="secondary" onClick={onClose}>
          Cancel
        </Button>
        <Button disabled={!date} onClick={() => void save(fromLocalInput(date))}>
          Save
        </Button>
      </DialogFooter>
    </Dialog>
  );
}

const REASONS: Record<RemovedMember['reason'], string> = { removed: 'removed', expired: 'membership ended', inactive: 'inactive' };

export function RemovedMembers({ removed, onRestored }: { removed: RemovedMember[]; onRestored: () => void }) {
  const [err, setErr] = useState<string | null>(null);
  if (!removed.length) return null;
  const restore = async (id: string) => {
    setErr(null);
    try {
      await patch({ action: 'restore', userIds: [id] });
      onRestored();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Card className="mt-4">
      <h3 className="mb-2 text-sm font-semibold">Recently removed</h3>
      <ErrorBanner message={err} />
      <ul className="divide-y divide-border">
        {removed.map((r) => (
          <li key={r.userId} className="flex items-center justify-between py-2 text-sm">
            <span>
              {r.name} ({r.email}){' '}
              <span className="text-xs text-foreground-tertiary">
                · {ROLE_LABELS[r.role]} · {REASONS[r.reason]} {formatRelativeTime(r.removedAt)} by {r.removedBy}
                {r.accountDisabled ? ' · account disabled' : ''} · restorable until {r.purgeAfter.slice(0, 10)}
              </span>
            </span>
            <Button size="sm" variant="ghost" onClick={() => void restore(r.userId)}>
              Restore
            </Button>
          </li>
        ))}
      </ul>
    </Card>
  );
}
