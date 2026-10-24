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
// display title; add new docs here as user-facing content is written. The
// docs route, the sitemap of /docs and the search index all read this list,
// so a page is searchable exactly when it is published.
const PUBLISHED_DOCS: DocEntry[] = [
  { slug: 'user-guide', title: 'User guide', description: 'Install, connect storage, transfer, sync, search and everyday use of the desktop app.', filename: 'user-guide.md' },
  { slug: 'cli-reference', title: 'CLI reference', description: 'Every ufop command, option and exit code.', filename: 'cli-reference.md' },
  { slug: 'troubleshooting', title: 'Troubleshooting', description: 'Diagnose and fix the most common problems.', filename: 'troubleshooting.md' },
  { slug: 'migration-guide', title: 'Migration guide', description: 'Move from other file managers and sync tools.', filename: 'migration-guide.md' },
  { slug: 'silent-install', title: 'Silent installation and fleet enrollment', description: 'Deploy the desktop app unattended and enroll machines in the admin console.', filename: 'silent-install.md' },
  { slug: 'licensing', title: 'Licensing', description: 'The client and console licensing split.', filename: 'licensing.md' },
  { slug: 'backup-sources-and-runs', title: 'Backup sources and runs', description: 'Ignore files, read errors, rehash, command output, upload limits and snapshots.', filename: 'backup-sources-and-runs.md' },
  { slug: 'backup-destinations', title: 'Backup destinations', description: 'How to set up each backup destination.', filename: 'backup-destinations.md' },
  { slug: 'backup-policies-and-schedules', title: 'Backup policies, schedules and maintenance', description: 'Schedules, retention policies and repository maintenance.', filename: 'backup-policies-and-schedules.md' },
  { slug: 'backup-restore', title: 'Restoring and reading backups', description: 'Browse versions, search and restore files.', filename: 'backup-restore.md' },
  { slug: 'backup-versions', title: 'Backup versions', description: 'Notes, tags, pins, retention and copies.', filename: 'backup-versions.md' },
  { slug: 'backup-encryption-and-secrets', title: 'Backup encryption and secrets', description: 'Encryption, certificate pinning and secret providers.', filename: 'backup-encryption-and-secrets.md' },
  { slug: 'backup-safety', title: 'Backup repository safety', description: 'Repository locks, the clock check, checkpoints and read-only connections.', filename: 'backup-safety.md' },
  { slug: 'backup-ransomware-hardening', title: 'Hardening backups against ransomware', description: 'Least-privilege credentials and the two-credential model.', filename: 'backup-ransomware-hardening.md' },
  { slug: 'backup-notifications', title: 'Backup notifications', description: 'Profiles, webhooks and HTML email.', filename: 'backup-notifications.md' },
  { slug: 'backup-logging-and-support', title: 'Backup logging and support tools', description: 'Notifications, log outputs, diagnostics and bug reports.', filename: 'backup-logging-and-support.md' },
  { slug: 'backup-photos', title: 'Backing up a macOS Photos library', description: 'Back up Photos libraries safely.', filename: 'backup-photos.md' },
  { slug: 'admin-guide', title: 'Admin console guide', description: 'Sign in, roles and the dashboard of the admin console.', filename: 'admin-guide.md' },
  { slug: 'admin-console', title: 'Admin console: fleet, tenancy and enterprise', description: 'Fleet, organizations, alerts, billing and enterprise features in detail.', filename: 'admin-console.md' },
  { slug: 'server-operations', title: 'Operating ufop-server', description: 'Accounts, roles, API tokens, sessions, tracing and logs of the headless server.', filename: 'server-operations.md' },
];

/** Where unpublished repo documents are linked from published ones. */
export const DOCS_REPO_BLOB_URL = 'https://github.com/ClappeDiya/file-manager/blob/master';

/**
 * Rewrites a link found in a published document's Markdown for the website. Docs link to each other with
 * repo-relative paths (`backup-safety.md#fragment`, `./user-guide.md`); on the site those would 404.
 * A relative `.md` link to a published document becomes `/docs/<slug>[#fragment]`; one to a document that
 * is not published goes to its page in the GitHub repository. Anything else (external, `#fragment`,
 * absolute path, non-Markdown) is returned unchanged.
 */
export function resolveDocHref(href: string): string {
  if (/^[a-z][a-z0-9+.-]*:/i.test(href) || href.startsWith('#') || href.startsWith('/')) return href;
  const hash = href.indexOf('#');
  const fragment = hash === -1 ? '' : href.slice(hash);
  const target = (hash === -1 ? href : href.slice(0, hash)).replace(/\?.*$/, '');
  if (!/\.md$/i.test(target)) return href;
  const repoPath = path.posix.normalize(path.posix.join('docs', target));
  if (repoPath.startsWith('../')) return href;
  const published = PUBLISHED_DOCS.find((d) => repoPath === `docs/${d.filename}`);
  return published ? `/docs/${published.slug}${fragment}` : `${DOCS_REPO_BLOB_URL}/${repoPath}${fragment}`;
}

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

/** Every published document with its Markdown source, for the search index. Documents that cannot be read are left out. */
export async function loadAllDocs(): Promise<{ entry: DocEntry; content: string }[]> {
  const loaded = await Promise.all(PUBLISHED_DOCS.map((d) => loadDoc(d.slug)));
  return loaded.filter((d): d is { entry: DocEntry; content: string } => d !== null);
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
