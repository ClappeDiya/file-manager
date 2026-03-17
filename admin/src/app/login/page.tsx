'use client';

import React, { useState, useEffect } from 'react';
import { useRouter } from 'next/navigation';
import { useAuthStore } from '@/lib/stores/auth-store';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Card, CardHeader, CardTitle, CardDescription, CardContent, CardFooter } from '@/components/ui/card';

export default function LoginPage() {
  const router = useRouter();
  const { isAuthenticated, isLoading, error, setError, setUser } = useAuthStore();
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [submitting, setSubmitting] = useState(false);

  useEffect(() => {
    if (isAuthenticated) {
      router.replace('/dashboard');
    }
  }, [isAuthenticated, router]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError(null);

    if (!email || !password) {
      setError('Please enter both email and password');
      return;
    }

    setSubmitting(true);
    try {
      const response = await fetch('/api/auth/login', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ email, password }),
      });

      const data = await response.json();

      if (!response.ok) {
        setError(data.error || 'Login failed');
        setSubmitting(false);
        return;
      }

      // Set user from the API response into the auth store
      setUser({
        id: 'usr_session',
        email: data.user.email,
        name: data.user.name,
        role: data.user.role === 'admin' ? 'org_owner' : 'user',
        permissions: [],
        organizationId: 'org_001',
        createdAt: new Date().toISOString(),
        isActive: true,
      });
    } catch {
      setError('Network error. Please try again.');
      setSubmitting(false);
    }
  };

  const handleSSOLogin = () => {
    // Placeholder for SAML SSO redirect
    setError('SSO is configured at the organization level. Contact your admin.');
  };

  return (
    <div className="min-h-screen flex items-center justify-center bg-background-secondary p-4">
      <div className="w-full max-w-sm">
        {/* Logo */}
        <div className="flex flex-col items-center mb-8">
          <div className="w-12 h-12 rounded-xl bg-primary flex items-center justify-center mb-3">
            <span className="text-primary-foreground font-bold text-xl">U</span>
          </div>
          <h1 className="text-xl font-bold text-foreground">UFOP Admin Console</h1>
          <p className="text-sm text-foreground-secondary mt-1">Unified File Operations Platform</p>
        </div>

        <Card>
          <CardHeader>
            <CardTitle>Sign in</CardTitle>
            <CardDescription>Enter your credentials to access the admin console</CardDescription>
          </CardHeader>
          <CardContent>
            <form onSubmit={handleSubmit} className="space-y-4">
              <Input
                label="Email"
                type="email"
                placeholder="admin@ufop.local"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                autoComplete="email"
                required
              />
              <Input
                label="Password"
                type="password"
                placeholder="Enter your password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                autoComplete="current-password"
                required
              />

              {error && (
                <div className="rounded-md bg-error-bg border border-error/20 px-3 py-2">
                  <p className="text-sm text-error">{error}</p>
                </div>
              )}

              <Button type="submit" className="w-full" loading={submitting || isLoading}>
                Sign in
              </Button>
            </form>
          </CardContent>
          <CardFooter className="flex-col gap-3">
            <div className="relative w-full">
              <div className="absolute inset-0 flex items-center">
                <div className="w-full border-t border-border" />
              </div>
              <div className="relative flex justify-center text-xs">
                <span className="bg-background-elevated px-2 text-foreground-tertiary">or</span>
              </div>
            </div>
            <Button variant="outline" className="w-full" onClick={handleSSOLogin}>
              Sign in with SSO (SAML)
            </Button>
          </CardFooter>
        </Card>

        <p className="text-xs text-center text-foreground-tertiary mt-4">
          UFOP Admin Console v0.1.0
        </p>
      </div>
    </div>
  );
}
