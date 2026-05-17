import {
  Folder,
  Network,
  ShieldCheck,
  RefreshCw,
  Bot,
  Lock,
  Terminal,
  History,
} from 'lucide-react';

const features = [
  {
    icon: Folder,
    title: 'Dual-pane browsing',
    body: 'Local and remote filesystems side by side. Tabs, dual sort, hidden-file toggle, and keyboard-driven navigation throughout.',
  },
  {
    icon: Network,
    title: '17 protocols, one app',
    body: 'SFTP, S3, Google Drive, Dropbox, OneDrive, WebDAV, SMB, NFS, FTP, B2, Azure Blob, Swift, mDNS peer, and more — all first-class.',
  },
  {
    icon: ShieldCheck,
    title: 'Crash-safe transfers',
    body: 'Three-layer engine: worker pool + checksums (xxHash3 / SHA-256) + journal-backed crash recovery. Resume any transfer at any time.',
  },
  {
    icon: RefreshCw,
    title: 'Real-time sync',
    body: 'Bidirectional sync with conflict resolution, redaction rules, and Merkle-tree integrity. Continuous or scheduled.',
  },
  {
    icon: Bot,
    title: 'Local AI assistant',
    body: 'Plain-language error explanations, sync-rule suggestions, and natural-language job creation. Runs against your own Ollama — no API keys, no cloud calls.',
  },
  {
    icon: Lock,
    title: 'Encryption + vault',
    body: 'AES-256-GCM at rest, master-password vault for credentials (stored in your OS keychain), per-job encryption for sensitive data.',
  },
  {
    icon: Terminal,
    title: 'Built-in terminal',
    body: 'Drop into a real shell scoped to the current directory. SSH sessions through the same connector you use to browse.',
  },
  {
    icon: History,
    title: 'Audit + version history',
    body: 'Every operation ledgered locally. Lineage tracking shows what a file is derived from. Undo across the entire session.',
  },
];

export function FeatureGrid() {
  return (
    <section id="features" className="marketing-section">
      <div className="container-wide">
        <div className="text-center max-w-3xl mx-auto mb-12">
          <h2 className="marketing-heading">Everything you need. None of the lock-in.</h2>
          <p className="marketing-subheading mt-4">
            One desktop app that handles browsing, transfer, sync, and governance — without sending your data through anyone's cloud.
          </p>
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-6">
          {features.map((feature) => (
            <div key={feature.title} className="marketing-card">
              <feature.icon size={24} className="text-primary mb-3" />
              <h3 className="font-semibold text-foreground mb-2">{feature.title}</h3>
              <p className="text-sm text-foreground-secondary leading-relaxed">{feature.body}</p>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
