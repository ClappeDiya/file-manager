'use client';

/** AI insights → Transcript: exactly what was sent to the model and what came back, checked against the audit chain. */
import React from 'react';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { Button } from '@/components/ui/button';
import { ErrorBanner } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import type { AiTranscript, TranscriptVerification } from '@/lib/ai';

export function TranscriptDialog({ reportId, onClose }: { reportId: string | null; onClose: () => void }) {
  const { data, error } = useApi<{ transcript: AiTranscript & { verification: TranscriptVerification } }>(reportId ? `/api/ai/transcripts/${reportId}` : null);
  if (!reportId) return null;
  const t = data?.transcript;
  const block = 'max-h-64 overflow-auto whitespace-pre-wrap rounded-md border border-border bg-background-secondary p-2 font-mono text-xs';
  return (
    <Dialog open onClose={onClose} className="max-w-3xl">
      <DialogHeader>
        <DialogTitle>AI transcript</DialogTitle>
        <DialogDescription>{t ? `${t.model} · recorded on the audit log as entry ${t.verification.auditSequence ?? '?'}` : 'Loading…'}</DialogDescription>
      </DialogHeader>
      <ErrorBanner message={error} />
      {t && (
        <div className="space-y-3 text-sm">
          <p className={t.verification.matches && t.verification.chainOk ? 'text-success' : 'text-error'}>
            {t.verification.matches && t.verification.chainOk ? '✓ Unchanged since it was recorded (hash on the audit chain, chain verifies)' : `✗ ${t.verification.reason}`}
          </p>
          <section>
            <h4 className="mb-1 font-semibold">System prompt</h4>
            <pre className={block}>{t.systemPrompt}</pre>
          </section>
          <section>
            <h4 className="mb-1 font-semibold">Request (instructions, question and fleet snapshot)</h4>
            <pre className={block}>{t.request}</pre>
          </section>
          <section>
            <h4 className="mb-1 font-semibold">Response</h4>
            <pre className={block}>{t.response}</pre>
          </section>
          <p className="break-all text-xs text-foreground-tertiary">SHA-256 {t.sha256}</p>
        </div>
      )}
      <DialogFooter>
        <Button variant="outline" onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
