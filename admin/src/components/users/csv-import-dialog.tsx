'use client';

/** Users → Import CSV: preview every row on the server, then invite. */
import React, { useEffect, useState } from 'react';
import { Button } from '@/components/ui/button';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Table, TableBody, TableCell, TableHead, TableHeader, TableRow } from '@/components/ui/table';
import { ErrorBanner, StatusBadge } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import type { ImportRow } from '@/lib/csv-import';

interface Result {
  dryRun: boolean;
  rows: ImportRow[];
  summary: { invited: number; skipped: number; errors: number; emailed?: number };
}

const BADGE: Record<ImportRow['action'], string> = { invite: 'pending', skip: 'cancelled', error: 'failed' };

export function CsvImportDialog({ csv, onClose, onImported }: { csv: string | null; onClose: () => void; onImported: () => void }) {
  const [result, setResult] = useState<Result | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  useEffect(() => {
    setResult(null);
    setErr(null);
    if (!csv) return;
    api<Result>('/api/users/import', { body: { csv, dryRun: true } })
      .then(setResult)
      .catch((e) => setErr(errorText(e)));
  }, [csv]);

  const run = async () => {
    setBusy(true);
    try {
      setResult(await api<Result>('/api/users/import', { body: { csv, dryRun: false } }));
      onImported();
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={csv !== null} onClose={onClose}>
      <DialogHeader>
        <DialogTitle>{result && !result.dryRun ? 'Import finished' : 'Import users from CSV'}</DialogTitle>
        <DialogDescription>
          Columns: <code>email</code> (required), <code>role</code>, <code>group</code>, <code>group_role</code>. Everyone receives an invitation; nobody gets a role above yours.
        </DialogDescription>
      </DialogHeader>
      <ErrorBanner message={err} />
      {result && (
        <div className="space-y-2">
          <p className="text-sm">
            {result.dryRun ? 'Will invite' : 'Invited'} {result.summary.invited}, skipped {result.summary.skipped}, errors {result.summary.errors}
            {result.summary.emailed !== undefined && ` · ${result.summary.emailed} e-mailed`}
          </p>
          <div className="max-h-80 overflow-auto">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead>Line</TableHead>
                  <TableHead>E-mail</TableHead>
                  <TableHead>Role / group</TableHead>
                  <TableHead>Result</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {result.rows.map((r) => (
                  <TableRow key={r.line}>
                    <TableCell className="text-xs">{r.line}</TableCell>
                    <TableCell className="text-xs">{r.email}</TableCell>
                    <TableCell className="text-xs">
                      {r.role}
                      {r.group && ` · ${r.group} (${r.groupRole})`}
                    </TableCell>
                    <TableCell className="text-xs">
                      <StatusBadge value={BADGE[r.action]} /> {r.message}
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </div>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          {result && !result.dryRun ? 'Close' : 'Cancel'}
        </Button>
        {result?.dryRun && (
          <Button disabled={busy || result.summary.invited === 0} onClick={() => void run()}>
            {busy ? 'Inviting…' : `Invite ${result.summary.invited}`}
          </Button>
        )}
      </DialogFooter>
    </Dialog>
  );
}
