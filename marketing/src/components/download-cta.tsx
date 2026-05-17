'use client';

import Link from 'next/link';
import { useEffect, useState } from 'react';
import { Apple, Monitor, Download } from 'lucide-react';

type Platform = 'macos' | 'windows' | 'linux' | 'unknown';

function detectPlatform(): Platform {
  if (typeof navigator === 'undefined') return 'unknown';
  const ua = navigator.userAgent.toLowerCase();
  if (ua.includes('mac')) return 'macos';
  if (ua.includes('win')) return 'windows';
  if (ua.includes('linux') || ua.includes('x11')) return 'linux';
  return 'unknown';
}

const platformLabels: Record<Platform, string> = {
  macos: 'macOS',
  windows: 'Windows',
  linux: 'Linux',
  unknown: 'your platform',
};

export function DownloadCta({ size = 'lg' }: { size?: 'md' | 'lg' }) {
  const [platform, setPlatform] = useState<Platform>('unknown');

  useEffect(() => {
    setPlatform(detectPlatform());
  }, []);

  const sizeClasses = size === 'lg' ? 'px-6 py-3 text-base' : 'px-4 py-2 text-sm';
  const Icon = platform === 'macos' ? Apple : platform === 'windows' ? Monitor : Download;

  return (
    <div className="flex flex-col sm:flex-row items-center gap-3">
      <Link
        href="/download"
        className={`inline-flex items-center gap-2 rounded-md bg-primary text-primary-foreground font-medium hover:bg-primary-hover transition-colors ${sizeClasses}`}
      >
        <Icon size={18} />
        Download for {platformLabels[platform]}
      </Link>
      <Link
        href="/pricing"
        className={`inline-flex items-center gap-2 rounded-md border border-border bg-background text-foreground font-medium hover:bg-background-secondary transition-colors ${sizeClasses}`}
      >
        See pricing
      </Link>
    </div>
  );
}
