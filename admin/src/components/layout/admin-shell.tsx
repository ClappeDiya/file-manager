'use client';

import React, { useEffect } from 'react';
import { useAuthStore } from '@/lib/stores/auth-store';
import { Sidebar } from './sidebar';
import { Header } from './header';

interface AdminShellProps {
  children: React.ReactNode;
}

export function AdminShell({ children }: AdminShellProps) {
  const { isAuthenticated, isValidating, validateSession } = useAuthStore();

  useEffect(() => {
    validateSession();
  }, [validateSession]);

  if (isValidating) {
    return (
      <div className="flex h-screen items-center justify-center bg-background">
        <div className="text-muted-foreground">Validating session...</div>
      </div>
    );
  }

  if (!isAuthenticated) {
    return <>{children}</>;
  }

  return (
    <div className="flex h-screen overflow-hidden bg-background">
      <Sidebar />
      <div className="flex flex-1 flex-col overflow-hidden">
        <Header />
        <main className="flex-1 overflow-y-auto p-4 lg:p-6">
          {children}
        </main>
      </div>
    </div>
  );
}
