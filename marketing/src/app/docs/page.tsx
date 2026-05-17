import type { Metadata } from 'next';
import Link from 'next/link';
import { BookOpen, Github, Terminal } from 'lucide-react';
import { listDocs } from '@/lib/docs';

export const metadata: Metadata = {
  title: 'Documentation',
  description: 'User guides, CLI reference, and connector setup for FileManager.',
};

export default async function DocsIndexPage() {
  const docs = await listDocs();

  return (
    <div className="container-page py-16">
      <header className="max-w-3xl mb-12">
        <h1 className="marketing-heading">Documentation</h1>
        <p className="marketing-subheading mt-4">
          Guides, references, and how-tos for getting the most out of FileManager.
        </p>
      </header>

      {docs.length === 0 ? (
        <div className="marketing-card max-w-3xl">
          <BookOpen size={28} className="text-primary mb-3" />
          <h2 className="font-semibold text-foreground mb-2">User docs land with v1.0</h2>
          <p className="text-sm text-foreground-secondary leading-relaxed mb-5">
            We are finalizing the public documentation alongside the v1.0 release. Until then, the in-app help
            (press <code className="text-xs px-1.5 py-0.5 rounded bg-background-secondary border border-border">Cmd</code> +
            <code className="text-xs px-1.5 py-0.5 rounded bg-background-secondary border border-border">?</code>)
            is the most complete reference, and the source repository contains every connector and command in detail.
          </p>
          <div className="flex flex-wrap gap-3">
            <Link
              href="/download"
              className="inline-flex items-center gap-2 px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium hover:bg-primary-hover transition-colors"
            >
              <Terminal size={16} />
              Download &amp; explore
            </Link>
            <a
              href="https://github.com/ufop/unified-file-ops"
              rel="noopener"
              className="inline-flex items-center gap-2 px-4 py-2 rounded-md border border-border bg-background text-foreground text-sm font-medium hover:bg-background-secondary transition-colors"
            >
              <Github size={16} />
              Browse the source
            </a>
          </div>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          {docs.map((doc) => (
            <Link key={doc.slug} href={`/docs/${doc.slug}`} className="marketing-card hover:border-border-hover transition-colors">
              <h2 className="font-semibold text-foreground mb-2">{doc.title}</h2>
              {doc.description && (
                <p className="text-sm text-foreground-secondary leading-relaxed">{doc.description}</p>
              )}
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}
