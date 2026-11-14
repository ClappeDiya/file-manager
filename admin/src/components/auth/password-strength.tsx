'use client';

/** Strength meter and the policy's problems for a new password. */
import React from 'react';
import { checkPassword } from '@/lib/password-policy';

const LABEL = ['Too weak', 'Weak', 'Fair', 'Good', 'Strong'];
const COLOR = ['bg-error', 'bg-error', 'bg-warning', 'bg-success', 'bg-success'];

export function PasswordStrength({ password, email, name }: { password: string; email?: string; name?: string }) {
  if (!password) return null;
  const c = checkPassword(password, { email, name });
  return (
    <div className="space-y-1" aria-live="polite" data-testid="password-strength">
      <div className="flex gap-1" aria-hidden="true">
        {[0, 1, 2, 3].map((i) => (
          <span key={i} className={`h-1 flex-1 rounded ${i < Math.max(1, c.score) ? COLOR[c.score] : 'bg-border'}`} />
        ))}
      </div>
      <p className="text-xs text-foreground-secondary">{LABEL[c.score]}</p>
      {c.problems.map((p) => (
        <p key={p} className="text-xs text-error">
          {p}
        </p>
      ))}
    </div>
  );
}
