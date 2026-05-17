'use client';

import { useState, FormEvent } from 'react';
import { CheckCircle2, AlertCircle } from 'lucide-react';

type Status = 'idle' | 'submitting' | 'success' | 'error';

export function WaitlistForm() {
  const [email, setEmail] = useState('');
  const [company, setCompany] = useState('');
  const [seats, setSeats] = useState('');
  const [status, setStatus] = useState<Status>('idle');
  const [error, setError] = useState<string | null>(null);

  async function handleSubmit(event: FormEvent) {
    event.preventDefault();
    setStatus('submitting');
    setError(null);

    try {
      const response = await fetch('/api/waitlist', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, company, seats }),
      });
      if (!response.ok) {
        const body = await response.json().catch(() => ({}));
        throw new Error(body.error ?? 'Could not join the waitlist. Please try again.');
      }
      setStatus('success');
      setEmail('');
      setCompany('');
      setSeats('');
    } catch (err) {
      setStatus('error');
      setError(err instanceof Error ? err.message : 'Unexpected error.');
    }
  }

  if (status === 'success') {
    return (
      <div id="waitlist" className="marketing-card text-center">
        <CheckCircle2 size={32} className="text-success mx-auto mb-3" />
        <h3 className="font-semibold text-foreground mb-1">You're on the list.</h3>
        <p className="text-sm text-foreground-secondary">
          We'll email you when Business is ready. No marketing fluff, just one announcement.
        </p>
      </div>
    );
  }

  return (
    <form id="waitlist" onSubmit={handleSubmit} className="marketing-card space-y-4">
      <div>
        <h3 className="font-semibold text-foreground">Get Business early</h3>
        <p className="text-sm text-foreground-secondary mt-1">
          Tell us about your team and we'll reach out before public launch with early-adopter pricing.
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
        <input
          type="email"
          required
          placeholder="Work email"
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          className="px-3 py-2 rounded-md border border-border bg-background text-foreground text-sm focus:outline-none focus:border-border-focus"
        />
        <input
          type="text"
          placeholder="Company (optional)"
          value={company}
          onChange={(e) => setCompany(e.target.value)}
          className="px-3 py-2 rounded-md border border-border bg-background text-foreground text-sm focus:outline-none focus:border-border-focus"
        />
        <input
          type="text"
          placeholder="Team size"
          value={seats}
          onChange={(e) => setSeats(e.target.value)}
          className="px-3 py-2 rounded-md border border-border bg-background text-foreground text-sm focus:outline-none focus:border-border-focus"
        />
      </div>

      {status === 'error' && error && (
        <div className="flex items-start gap-2 text-sm text-error">
          <AlertCircle size={16} className="shrink-0 mt-0.5" />
          <span>{error}</span>
        </div>
      )}

      <button
        type="submit"
        disabled={status === 'submitting'}
        className="px-5 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium hover:bg-primary-hover transition-colors disabled:opacity-60"
      >
        {status === 'submitting' ? 'Submitting…' : 'Join the waitlist'}
      </button>

      <p className="text-xs text-foreground-tertiary">
        We'll only email you about Business availability. No newsletter, no third-party sharing.
      </p>
    </form>
  );
}
