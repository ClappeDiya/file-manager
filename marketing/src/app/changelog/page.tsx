import { DocMarkdown } from '@/components/doc-markdown';
import type { Metadata } from 'next';
import fs from 'node:fs/promises';
import path from 'node:path';

export const metadata: Metadata = {
  title: 'Changelog',
  description: 'Every FileManager release, in chronological order.',
};

async function loadChangelog(): Promise<string> {
  const candidates = [
    path.join(process.cwd(), '..', 'CHANGELOG.md'),
    path.join(process.cwd(), 'CHANGELOG.md'),
  ];
  for (const candidate of candidates) {
    try {
      return await fs.readFile(candidate, 'utf8');
    } catch {
      continue;
    }
  }
  return '_No changelog yet — check back after the first release._';
}

export default async function ChangelogPage() {
  // The page header already supplies the document title. Keep release headings
  // at their original levels without publishing a second h1 from the source.
  const markdown = (await loadChangelog()).replace(/^# Changelog\s*\n/, '');

  return (
    <div className="container-prose py-16">
      <header className="mb-10">
        <h1 className="marketing-heading">Changelog</h1>
        <p className="marketing-subheading mt-3">
          Release notes, fixes and known changes.
        </p>
      </header>

      <article className="prose prose-sm dark:prose-invert max-w-none">
        <DocMarkdown content={markdown} />
      </article>
    </div>
  );
}
