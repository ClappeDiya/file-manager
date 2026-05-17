'use client';

import Link from 'next/link';
import { useState } from 'react';
import { Menu, X, Download } from 'lucide-react';

const navLinks = [
  { href: '/#features', label: 'Features' },
  { href: '/#connectors', label: 'Connectors' },
  { href: '/pricing', label: 'Pricing' },
  { href: '/docs', label: 'Docs' },
  { href: '/changelog', label: 'Changelog' },
];

export function SiteNav() {
  const [open, setOpen] = useState(false);

  return (
    <header className="sticky top-0 z-40 w-full border-b border-border bg-background/85 backdrop-blur supports-[backdrop-filter]:bg-background/70">
      <div className="container-wide flex h-16 items-center justify-between">
        <Link href="/" className="flex items-center gap-2 font-semibold text-foreground hover:text-foreground">
          <div className="w-8 h-8 rounded-md bg-primary text-primary-foreground grid place-items-center font-bold">F</div>
          <span>FileManager</span>
          <span className="pill ml-2">by Clappe</span>
        </Link>

        <nav className="hidden md:flex items-center gap-7 text-sm">
          {navLinks.map((link) => (
            <Link key={link.href} href={link.href} className="text-foreground-secondary hover:text-foreground transition-colors">
              {link.label}
            </Link>
          ))}
        </nav>

        <div className="hidden md:flex items-center gap-3">
          <Link
            href="/download"
            className="inline-flex items-center gap-2 px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium hover:bg-primary-hover transition-colors"
          >
            <Download size={16} />
            Download
          </Link>
        </div>

        <button
          type="button"
          className="md:hidden p-2 rounded-md text-foreground-secondary hover:bg-background-secondary"
          onClick={() => setOpen(!open)}
          aria-label="Toggle menu"
        >
          {open ? <X size={20} /> : <Menu size={20} />}
        </button>
      </div>

      {open && (
        <div className="md:hidden border-t border-border bg-background">
          <nav className="container-wide py-4 flex flex-col gap-3 text-sm">
            {navLinks.map((link) => (
              <Link
                key={link.href}
                href={link.href}
                onClick={() => setOpen(false)}
                className="py-2 text-foreground-secondary hover:text-foreground"
              >
                {link.label}
              </Link>
            ))}
            <Link
              href="/download"
              onClick={() => setOpen(false)}
              className="mt-2 inline-flex items-center justify-center gap-2 px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium"
            >
              <Download size={16} />
              Download
            </Link>
          </nav>
        </div>
      )}
    </header>
  );
}
