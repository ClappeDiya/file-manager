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
    title: 'Cloud and server connections',
    body: 'SFTP, S3, Google Drive, Dropbox, OneDrive, WebDAV, SMB, NFS, FTP, B2, Azure Blob, Swift, mDNS peer, and more; supported operations vary by service.',
  },
  {
    icon: ShieldCheck,
    title: 'Crash-safe transfers',
    body: 'Three-layer engine: worker pool + checksums (xxHash3 / SHA-256) + journal-backed crash recovery. Local file copies verify checksums and support pause, resume and cancellation. Remote recovery depends on the provider.',
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
    body: 'Every operation ledgered locally. Lineage tracking shows what a file is derived from. Undo is available for supported operations.',
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
          {features.slice(0, 4).map((feature) => (
            <div key={feature.title} className="marketing-card">
              <feature.icon size={24} className="text-primary mb-3" />
              <h3 className="font-semibold text-foreground mb-2">{feature.title}</h3>
              <p className="text-sm text-foreground-secondary leading-relaxed">{feature.body}</p>
            </div>
          ))}
        </div>
        <details className="mt-6">
          <summary className="cursor-pointer py-3 text-primary font-medium">Explore AI, encryption, terminal and history</summary>
          <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mt-4">{features.slice(4).map((feature) => <div key={feature.title}><h3 className="font-semibold text-foreground">{feature.title}</h3><p className="text-sm text-foreground-secondary mt-2">{feature.body}</p></div>)}</div>
        </details>
      </div>
    </section>
  );
}
