import type { Metadata, Viewport } from 'next';
// Single source of truth for theme tokens — same file used by the desktop app
// so any color / spacing / typography change in the shared package propagates.
import '@ufop/design-tokens/css';
import '@/styles/globals.css';
import { Providers } from './providers';

export const metadata: Metadata = {
  title: 'UFOP Admin Console',
  description: 'Unified File Operations Platform - Administration Console',
};

export const viewport: Viewport = {
  width: 'device-width',
  initialScale: 1,
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="light" suppressHydrationWarning>
      <body>
        <Providers>{children}</Providers>
      </body>
    </html>
  );
}
