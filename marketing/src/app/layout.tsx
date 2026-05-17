import type { Metadata, Viewport } from 'next';
// Single source of truth for theme tokens — same file the desktop app and
// admin console use, so any color/spacing/typography change propagates.
import '@ufop/design-tokens/css';
import '@/styles/globals.css';
import { SiteNav } from '@/components/site-nav';
import { SiteFooter } from '@/components/site-footer';

export const metadata: Metadata = {
  metadataBase: new URL('https://filemanager.clappe.com'),
  title: {
    default: 'FileManager — The cross-platform file manager for power users',
    template: '%s · FileManager',
  },
  description:
    'Local + remote browsing, 17 protocols, crash-safe transfers, real-time sync, and a local AI assistant. Free for individuals. Cross-platform — macOS, Windows, Linux.',
  openGraph: {
    type: 'website',
    siteName: 'FileManager',
    url: 'https://filemanager.clappe.com',
  },
  twitter: {
    card: 'summary_large_image',
  },
};

export const viewport: Viewport = {
  width: 'device-width',
  initialScale: 1,
  themeColor: [
    { media: '(prefers-color-scheme: light)', color: '#ffffff' },
    { media: '(prefers-color-scheme: dark)', color: '#0a0a0a' },
  ],
};

export default function RootLayout({
  children,
}: {
  children: React.ReactNode;
}) {
  return (
    <html lang="en" data-theme="light" suppressHydrationWarning>
      <body className="min-h-screen flex flex-col">
        <SiteNav />
        <main className="flex-1">{children}</main>
        <SiteFooter />
      </body>
    </html>
  );
}
