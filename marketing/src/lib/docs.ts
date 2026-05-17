import fs from 'node:fs/promises';
import path from 'node:path';

// User documentation lives in the repo-root `docs/` directory and is mirrored
// here at build time. Each .md file becomes a slug at /docs/<slug>.
//
// The repo-root docs/ also contains the PRD and internal governance docs that
// should NOT be on the public site; we use an allow-list rather than auto-
// publishing everything to keep that boundary explicit.

const DOCS_ROOT_CANDIDATES = [
  path.join(process.cwd(), '..', 'docs'),
  path.join(process.cwd(), 'docs'),
];

export type DocEntry = {
  slug: string;
  title: string;
  description?: string;
  filename: string;
};

// Allow-list of files to publish. Each entry maps a source filename to a
// display title; add new docs here as user-facing content is written.
const PUBLISHED_DOCS: DocEntry[] = [
  // {
  //   slug: 'getting-started',
  //   title: 'Getting started',
  //   description: 'Install FileManager and complete your first transfer in 5 minutes.',
  //   filename: 'getting-started.md',
  // },
];

async function findDocsRoot(): Promise<string | null> {
  for (const candidate of DOCS_ROOT_CANDIDATES) {
    try {
      const stat = await fs.stat(candidate);
      if (stat.isDirectory()) return candidate;
    } catch {
      continue;
    }
  }
  return null;
}

export async function listDocs(): Promise<DocEntry[]> {
  return PUBLISHED_DOCS;
}

export async function loadDoc(slug: string): Promise<{ entry: DocEntry; content: string } | null> {
  const entry = PUBLISHED_DOCS.find((d) => d.slug === slug);
  if (!entry) return null;

  const root = await findDocsRoot();
  if (!root) return null;

  try {
    const content = await fs.readFile(path.join(root, entry.filename), 'utf8');
    return { entry, content };
  } catch {
    return null;
  }
}
