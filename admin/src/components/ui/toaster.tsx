'use client';

import React from 'react';
import { X, AlertCircle, CheckCircle, Info } from 'lucide-react';
import { useToastStore, type ToastVariant } from '@/lib/stores/toast-store';
import { cn } from '@/lib/utils/cn';

const VARIANT_STYLES: Record<ToastVariant, { wrap: string; icon: React.ElementType }> = {
  success: { wrap: 'border-success/40 bg-success/10 text-success', icon: CheckCircle },
  error: { wrap: 'border-error/40 bg-error/10 text-error', icon: AlertCircle },
  info: { wrap: 'border-border bg-background-elevated text-foreground', icon: Info },
};

export function Toaster() {
  const toasts = useToastStore((s) => s.toasts);
  const dismiss = useToastStore((s) => s.dismiss);

  if (toasts.length === 0) return null;

  return (
    <div
      className="fixed bottom-4 right-4 z-[100] flex flex-col gap-2 w-80 max-w-[calc(100vw-2rem)]"
      role="region"
      aria-label="Notifications"
      aria-live="polite"
    >
      {toasts.map((toast) => {
        const { wrap, icon: Icon } = VARIANT_STYLES[toast.variant];
        return (
          <div
            key={toast.id}
            className={cn('flex items-start gap-2 rounded-md border p-3 shadow-md', wrap)}
            role={toast.variant === 'error' ? 'alert' : 'status'}
          >
            <Icon size={16} className="mt-0.5 shrink-0" aria-hidden="true" />
            <p className="flex-1 text-sm leading-snug">{toast.message}</p>
            <button
              onClick={() => dismiss(toast.id)}
              className="opacity-70 hover:opacity-100"
              aria-label="Dismiss notification"
            >
              <X size={14} aria-hidden="true" />
            </button>
          </div>
        );
      })}
    </div>
  );
}
