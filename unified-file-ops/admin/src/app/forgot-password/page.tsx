'use client';

import React, { useState } from 'react';
import { useRouter } from 'next/navigation';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent } from '@/components/ui/card';

type RecoveryMethod = 'choose' | 'code' | 'email';

export default function ForgotPasswordPage() {
  const router = useRouter();
  const [method, setMethod] = useState<RecoveryMethod>('choose');
  const [recoveryCode, setRecoveryCode] = useState('');
  const [email, setEmail] = useState('');
  const [newPassword, setNewPassword] = useState('');
  const [confirmPassword, setConfirmPassword] = useState('');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [success, setSuccess] = useState<string | null>(null);
  const [smtpAvailable, setSmtpAvailable] = useState(false);

  // Check if SMTP is configured on mount
  React.useEffect(() => {
    fetch('/api/auth/forgot-password', { method: 'OPTIONS' })
      .then((r) => r.json())
      .then((data) => setSmtpAvailable(data.smtpConfigured === true))
      .catch(() => setSmtpAvailable(false));
  }, []);

  const handleCodeReset = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!recoveryCode.trim()) {
      setError('Please enter a recovery code');
      return;
    }
    if (!newPassword || newPassword.length < 8) {
      setError('New password must be at least 8 characters');
      return;
    }
    if (newPassword !== confirmPassword) {
      setError('Passwords do not match');
      return;
    }

    setSubmitting(true);
    try {
      const response = await fetch('/api/auth/reset-password', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          method: 'code',
          recoveryCode: recoveryCode.trim(),
          newPassword,
        }),
      });

      const data = await response.json();
      if (!response.ok) {
        setError(data.error || 'Reset failed');
      } else {
        setSuccess('Password reset successfully. You can now sign in with your new password.');
        setRecoveryCode('');
        setNewPassword('');
        setConfirmPassword('');
      }
    } catch {
      setError('Network error. Please try again.');
    } finally {
      setSubmitting(false);
    }
  };

  const handleEmailReset = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!email.trim()) {
      setError('Please enter your email address');
      return;
    }

    setSubmitting(true);
    try {
      const response = await fetch('/api/auth/forgot-password', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email: email.trim() }),
      });

      const data = await response.json();
      if (!response.ok) {
        setError(data.error || 'Failed to send reset email');
      } else {
        setSuccess('If that email is registered, a reset link has been sent. Check your inbox.');
      }
    } catch {
      setError('Network error. Please try again.');
    } finally {
      setSubmitting(false);
    }
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-background-secondary p-4">
      <div className="w-full max-w-sm">
        {/* Logo */}
        <div className="flex flex-col items-center mb-8">
          <div className="w-12 h-12 rounded-xl bg-primary flex items-center justify-center mb-3">
            <span className="text-primary-foreground font-bold text-xl">U</span>
          </div>
          <h1 className="text-xl font-bold text-foreground">Reset Password</h1>
          <p className="text-sm text-foreground-secondary mt-1">UFOP Admin Console</p>
        </div>

        {success ? (
          <Card>
            <CardContent>
              <div className="rounded-md bg-success-bg border border-success/20 px-3 py-3 mb-4">
                <p className="text-sm text-success-foreground">{success}</p>
              </div>
              <Button className="w-full" onClick={() => router.push('/login')}>
                Back to Sign In
              </Button>
            </CardContent>
          </Card>
        ) : method === 'choose' ? (
          <Card>
            <CardHeader>
              <CardTitle>Recovery Method</CardTitle>
              <CardDescription>Choose how to reset your password</CardDescription>
            </CardHeader>
            <CardContent>
              <div className="space-y-3">
                <button
                  onClick={() => setMethod('code')}
                  className="w-full rounded-lg border border-border hover:border-primary/50 hover:bg-background-tertiary px-4 py-3 text-left transition-colors"
                >
                  <div className="flex items-center gap-3">
                    <span className="text-lg">🔑</span>
                    <div>
                      <p className="text-sm font-medium text-foreground">Use Recovery Code</p>
                      <p className="text-xs text-foreground-secondary">Enter one of your saved recovery codes</p>
                    </div>
                  </div>
                </button>

                {smtpAvailable && (
                  <button
                    onClick={() => setMethod('email')}
                    className="w-full rounded-lg border border-border hover:border-primary/50 hover:bg-background-tertiary px-4 py-3 text-left transition-colors"
                  >
                    <div className="flex items-center gap-3">
                      <span className="text-lg">📧</span>
                      <div>
                        <p className="text-sm font-medium text-foreground">Email Reset Link</p>
                        <p className="text-xs text-foreground-secondary">Send a reset link to your registered email</p>
                      </div>
                    </div>
                  </button>
                )}

                <div className="pt-2">
                  <button
                    onClick={() => router.push('/login')}
                    className="text-xs text-primary hover:underline"
                  >
                    ← Back to Sign In
                  </button>
                </div>
              </div>
            </CardContent>
          </Card>
        ) : method === 'code' ? (
          <Card>
            <CardHeader>
              <CardTitle>Recovery Code</CardTitle>
              <CardDescription>Enter one of the recovery codes you saved during setup</CardDescription>
            </CardHeader>
            <CardContent>
              <form onSubmit={handleCodeReset} className="space-y-4">
                <Input
                  label="Recovery Code"
                  type="text"
                  placeholder="XXXX-XXXX-XXXX"
                  value={recoveryCode}
                  onChange={(e) => setRecoveryCode(e.target.value)}
                  required
                />
                <Input
                  label="New Password"
                  type="password"
                  placeholder="At least 8 characters"
                  value={newPassword}
                  onChange={(e) => setNewPassword(e.target.value)}
                  required
                />
                <Input
                  label="Confirm Password"
                  type="password"
                  placeholder="Confirm your new password"
                  value={confirmPassword}
                  onChange={(e) => setConfirmPassword(e.target.value)}
                  required
                />

                {error && (
                  <div className="rounded-md bg-error-bg border border-error/20 px-3 py-2">
                    <p className="text-sm text-error">{error}</p>
                  </div>
                )}

                <Button type="submit" className="w-full" loading={submitting}>
                  Reset Password
                </Button>

                <div className="text-center">
                  <button
                    type="button"
                    onClick={() => { setMethod('choose'); setError(null); }}
                    className="text-xs text-primary hover:underline"
                  >
                    ← Choose different method
                  </button>
                </div>
              </form>
            </CardContent>
          </Card>
        ) : (
          <Card>
            <CardHeader>
              <CardTitle>Email Reset</CardTitle>
              <CardDescription>We&apos;ll send a password reset link to your email</CardDescription>
            </CardHeader>
            <CardContent>
              <form onSubmit={handleEmailReset} className="space-y-4">
                <Input
                  label="Email"
                  type="email"
                  placeholder="admin@yourorg.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  required
                />

                {error && (
                  <div className="rounded-md bg-error-bg border border-error/20 px-3 py-2">
                    <p className="text-sm text-error">{error}</p>
                  </div>
                )}

                <Button type="submit" className="w-full" loading={submitting}>
                  Send Reset Link
                </Button>

                <div className="text-center">
                  <button
                    type="button"
                    onClick={() => { setMethod('choose'); setError(null); }}
                    className="text-xs text-primary hover:underline"
                  >
                    ← Choose different method
                  </button>
                </div>
              </form>
            </CardContent>
          </Card>
        )}

        <p className="text-xs text-center text-foreground-tertiary mt-4">
          UFOP Admin Console v0.1.0
        </p>
      </div>
    </div>
  );
}
