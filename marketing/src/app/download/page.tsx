import type { Metadata } from 'next';
import { Apple, Monitor, Server, Terminal, ShieldCheck, AlertCircle } from 'lucide-react';

export const metadata: Metadata = {
  title: 'Download',
  description: 'Get FileManager for macOS, Windows, or Linux. Signed installers with auto-updates.',
};

export const revalidate = 300;

const REPO = 'ClappeDiya/file-manager';

type GHAsset = { name: string; browser_download_url: string };
type GHRelease = {
  tag_name: string;
  name: string;
  published_at: string;
  html_url: string;
  prerelease: boolean;
  assets: GHAsset[];
};

type Download = {
  label: string;
  // Literal filename produced by scripts/release-local.sh. We try this first,
  // then fall back to the first asset whose name matches `pattern`.
  literal: string;
  pattern: RegExp;
};

type Platform = {
  id: string;
  title: string;
  icon: typeof Apple;
  requirement: string;
  downloads: Download[];
  code: string | null;
};

const platforms: Platform[] = [
  {
    id: 'macos',
    title: 'macOS',
    icon: Apple,
    requirement: 'macOS 10.15 Catalina or newer · Apple Silicon + Intel',
    downloads: [
      { label: 'Universal .dmg', literal: 'FileManager-latest-universal.dmg', pattern: /universal\.dmg$/i },
    ],
    code: 'brew install --cask ClappeDiya/tap/unified-file-ops',
  },
  {
    id: 'windows',
    title: 'Windows',
    icon: Monitor,
    requirement: 'Windows 10 build 1809+ or Windows 11 · x86_64',
    downloads: [
      { label: '.exe installer (NSIS)', literal: 'FileManager-latest-x64-setup.exe', pattern: /x64-setup\.exe$/i },
      { label: '.msi (WiX)', literal: 'FileManager-latest-x64.msi', pattern: /x64.*\.msi$/i },
    ],
    code: 'winget install UFOP.UnifiedFileOps',
  },
  {
    id: 'linux',
    title: 'Linux',
    icon: Server,
    requirement: 'glibc 2.31+ · x86_64 · GTK 3 + WebKit2GTK 4.1',
    downloads: [
      { label: '.AppImage', literal: 'FileManager-latest-amd64.AppImage', pattern: /\.AppImage$/i },
      { label: '.deb (Debian / Ubuntu)', literal: 'FileManager-latest-amd64.deb', pattern: /\.deb$/i },
      { label: '.rpm (Fedora / RHEL)', literal: 'FileManager-latest-amd64.rpm', pattern: /\.rpm$/i },
    ],
    code: null,
  },
];

async function fetchLatestRelease(): Promise<GHRelease | null> {
  try {
    const r = await fetch(`https://api.github.com/repos/${REPO}/releases/latest`, {
      headers: { Accept: 'application/vnd.github+json', 'User-Agent': 'filemanager.clappe.com' },
      next: { revalidate: 300 },
    });
    if (!r.ok) return null;
    return (await r.json()) as GHRelease;
  } catch {
    return null;
  }
}

function resolveHref(d: Download, release: GHRelease | null): string | null {
  if (!release) return null;
  const literal = release.assets.find((a) => a.name === d.literal);
  if (literal) return literal.browser_download_url;
  const fallback = release.assets.find((a) => d.pattern.test(a.name));
  return fallback ? fallback.browser_download_url : null;
}

export default async function DownloadPage() {
  const release = await fetchLatestRelease();

  return (
    <div className="container-wide py-16">
      <header className="max-w-3xl mb-12">
        <h1 className="marketing-heading">Download FileManager</h1>
        <p className="marketing-subheading mt-4">
          Free. Signed. Auto-updating. Pick your OS below — every installer is code-signed and verified before release.
        </p>
        {!release && (
          <div className="mt-6 flex items-start gap-3 rounded-md border border-warning bg-warning-bg p-4">
            <AlertCircle size={18} className="mt-0.5 shrink-0 text-warning" />
            <div className="text-sm">
              <p className="font-medium text-warning-foreground">Pre-release — installers coming soon.</p>
              <p className="mt-1 text-warning-foreground">
                Signed binaries are being prepared for the first public release. Star{' '}
                <a href={`https://github.com/${REPO}`} className="underline" rel="noopener">
                  the repo
                </a>{' '}
                or check back here to be notified.
              </p>
            </div>
          </div>
        )}
      </header>

      <div className="grid grid-cols-1 md:grid-cols-3 gap-6 mb-12">
        {platforms.map((platform) => (
          <section key={platform.id} className="marketing-card">
            <div className="flex items-center gap-3 mb-3">
              <platform.icon size={28} className="text-primary" />
              <h2 className="text-xl font-bold text-foreground">{platform.title}</h2>
            </div>
            <p className="text-xs text-foreground-tertiary mb-5">{platform.requirement}</p>

            <ul className="space-y-2 mb-5">
              {platform.downloads.map((dl) => {
                const href = resolveHref(dl, release);
                if (href) {
                  return (
                    <li key={dl.label}>
                      <a
                        href={href}
                        className="block w-full text-center px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium hover:bg-primary-hover transition-colors"
                        rel="noopener"
                      >
                        {dl.label}
                      </a>
                    </li>
                  );
                }
                return (
                  <li key={dl.label}>
                    <span
                      className="block w-full text-center px-4 py-2 rounded-md bg-background-secondary border border-border text-foreground-tertiary text-sm font-medium cursor-not-allowed"
                      aria-disabled="true"
                      title="Not yet published"
                    >
                      {dl.label} <span className="opacity-60">— coming soon</span>
                    </span>
                  </li>
                );
              })}
            </ul>

            {platform.code && (
              <div>
                <p className="text-xs text-foreground-tertiary mb-2 flex items-center gap-1">
                  <Terminal size={12} /> Package manager
                </p>
                <pre className="text-xs bg-background-secondary border border-border rounded-md p-3 font-mono overflow-x-auto">
                  <code>{platform.code}</code>
                </pre>
              </div>
            )}
          </section>
        ))}
      </div>

      <section className="marketing-card max-w-3xl">
        <div className="flex items-center gap-3 mb-3">
          <ShieldCheck className="text-success" size={20} />
          <h2 className="font-semibold text-foreground">Verify your download</h2>
        </div>
        <p className="text-sm text-foreground-secondary mb-3">
          Every release is signed with our Tauri updater key. Auto-updates verify the signature before installing.
          For manual verification, the SHA-256 checksums and signature files are attached to every GitHub release.
        </p>
        <a
          href={`https://github.com/${REPO}/releases`}
          className="text-sm font-medium"
          rel="noopener"
        >
          View all releases on GitHub →
        </a>
      </section>
    </div>
  );
}
