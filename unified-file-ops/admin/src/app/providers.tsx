'use client';

import React from 'react';
import { AdminShell } from '@/components/layout/admin-shell';

export function Providers({ children }: { children: React.ReactNode }) {
  return <AdminShell>{children}</AdminShell>;
}
