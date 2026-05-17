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
  const markdown = await loadChangelog();

  return (
    <div className="container-prose py-16">
      <header className="mb-10">
        <h1 className="marketing-heading">Changelog</h1>
        <p className="marketing-subheading mt-3">
          Every release, in the order it shipped. Subscribe via the RSS feed at <code className="text-sm">/changelog.xml</code> (coming soon).
        </p>
      </header>

      <article className="prose prose-sm dark:prose-invert max-w-none">
        <pre className="whitespace-pre-wrap font-sans text-sm leading-relaxed text-foreground-secondary">
          {markdown}
        </pre>
      </article>
    </div>
  );
}
