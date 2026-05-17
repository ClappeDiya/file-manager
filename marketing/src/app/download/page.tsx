import type { Metadata } from 'next';
import { Apple, Monitor, Server, Terminal, ShieldCheck } from 'lucide-react';

export const metadata: Metadata = {
  title: 'Download',
  description: 'Get FileManager for macOS, Windows, or Linux. Signed installers with auto-updates.',
};

const RELEASE_BASE = 'https://github.com/ufop/unified-file-ops/releases/latest/download';
const VERSION_LABEL = 'latest';

const platforms = [
  {
    id: 'macos',
    title: 'macOS',
    icon: Apple,
    requirement: 'macOS 10.15 Catalina or newer · Apple Silicon + Intel',
    downloads: [
      { label: 'Universal .dmg', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-universal.dmg` },
    ],
    code: 'brew install --cask ufop/tap/unified-file-ops',
  },
  {
    id: 'windows',
    title: 'Windows',
    icon: Monitor,
    requirement: 'Windows 10 build 1809+ or Windows 11 · x86_64',
    downloads: [
      { label: '.exe installer (NSIS)', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-x64-setup.exe` },
      { label: '.msi (WiX)', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-x64.msi` },
    ],
    code: 'winget install UFOP.UnifiedFileOps',
  },
  {
    id: 'linux',
    title: 'Linux',
    icon: Server,
    requirement: 'glibc 2.31+ · x86_64 · GTK 3 + WebKit2GTK 4.1',
    downloads: [
      { label: '.AppImage', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-amd64.AppImage` },
      { label: '.deb (Debian / Ubuntu)', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-amd64.deb` },
      { label: '.rpm (Fedora / RHEL)', href: `${RELEASE_BASE}/FileManager-${VERSION_LABEL}-amd64.rpm` },
    ],
    code: null,
  },
];

export default function DownloadPage() {
  return (
    <div className="container-wide py-16">
      <header className="max-w-3xl mb-12">
        <h1 className="marketing-heading">Download FileManager</h1>
        <p className="marketing-subheading mt-4">
          Free. Signed. Auto-updating. Pick your OS below — every installer is code-signed and verified before release.
        </p>
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
              {platform.downloads.map((dl) => (
                <li key={dl.label}>
                  <a
                    href={dl.href}
                    className="block w-full text-center px-4 py-2 rounded-md bg-primary text-primary-foreground text-sm font-medium hover:bg-primary-hover transition-colors"
                    rel="noopener"
                  >
                    {dl.label}
                  </a>
                </li>
              ))}
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
          href="https://github.com/ufop/unified-file-ops/releases"
          className="text-sm font-medium"
          rel="noopener"
        >
          View all releases on GitHub →
        </a>
      </section>
    </div>
  );
}
