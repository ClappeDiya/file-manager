'use client';

/** Settings → SSO → Diagnostics: recent sign-ins through a provider, step by step. */
import React, { useState } from 'react';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { formatDateTime } from '@/lib/utils/format';
import type { SsoAttempt } from '@/lib/db/repos/sso';

const STEP_LABELS: Record<string, string> = {
  request: 'Request',
  idp_response: 'Answer from the identity provider',
  state: 'Sign-in state',
  discovery: 'Provider discovery',
  token_exchange: 'Code exchange',
  token_verify: 'Identity token check',
  nonce: 'Nonce check',
  userinfo: 'User info',
  claims: 'Claims received',
  saml_validate: 'SAML response check',
  assertion: 'Assertion received',
  account: 'Console account',
  sign_in: 'Sign-in',
};

export function SsoDiagnostics({ provider, onClose }: { provider: { id: string; name: string } | null; onClose: () => void }) {
  const { data, error } = useApi<{ attempts: SsoAttempt[] }>(provider ? `/api/sso/${provider.id}/attempts` : null, 15_000);
  const [open, setOpen] = useState<string | null>(null);
  if (!provider) return null;
  return (
    <Dialog open onClose={onClose} className="max-w-3xl">
      <DialogHeader>
        <DialogTitle>Sign-in diagnostics — {provider.name}</DialogTitle>
        <DialogDescription>The last sign-ins through this provider, with what it sent at each step (never tokens or signatures).</DialogDescription>
      </DialogHeader>
      <ErrorBanner message={error} />
      <ul className="max-h-[60vh] space-y-2 overflow-y-auto text-sm">
        {data?.attempts.map((a) => (
          <li key={a.id} className="rounded-md border border-border p-2">
            <button className="flex w-full items-center justify-between gap-2 text-left" onClick={() => setOpen(open === a.id ? null : a.id)}>
              <span>
                <span className={a.ok ? 'text-success' : 'text-error'}>{a.ok ? '✓' : '✗'}</span> {a.email || 'unknown user'}
                {!a.ok && (
                  <span className="text-error">
                    {' '}
                    — {STEP_LABELS[a.stage] ?? a.stage}: {a.error}
                  </span>
                )}
              </span>
              <span className="shrink-0 text-xs text-foreground-tertiary">
                {formatDateTime(a.at)} · {a.ip}
              </span>
            </button>
            {open === a.id && (
              <ol className="mt-2 space-y-1 border-t border-border pt-2">
                {a.steps.map((s, i) => (
                  <li key={i}>
                    <span className={s.ok ? 'text-success' : 'text-error'}>{s.ok ? '✓' : '✗'}</span> {STEP_LABELS[s.step] ?? s.step}
                    {s.detail && <pre className="mt-1 overflow-x-auto rounded bg-background-secondary p-2 font-mono text-xs">{JSON.stringify(s.detail, null, 2)}</pre>}
                  </li>
                ))}
              </ol>
            )}
          </li>
        ))}
        {data && !data.attempts.length && <li className="text-foreground-tertiary">No sign-ins through this provider yet.</li>}
      </ul>
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
