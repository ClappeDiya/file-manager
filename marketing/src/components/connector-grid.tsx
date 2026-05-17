const connectors = [
  { name: 'Local Drive', kind: 'Local' },
  { name: 'SFTP', kind: 'SSH' },
  { name: 'FTP / FTPS', kind: 'Legacy' },
  { name: 'WebDAV', kind: 'HTTP' },
  { name: 'SMB / CIFS', kind: 'Network' },
  { name: 'NFS', kind: 'Network' },
  { name: 'Amazon S3', kind: 'Cloud' },
  { name: 'Google Drive', kind: 'Cloud' },
  { name: 'Dropbox', kind: 'Cloud' },
  { name: 'OneDrive', kind: 'Cloud' },
  { name: 'Backblaze B2', kind: 'Cloud' },
  { name: 'Azure Blob', kind: 'Cloud' },
  { name: 'OpenStack Swift', kind: 'Cloud' },
  { name: 'S3-compatible', kind: 'Cloud' },
  { name: 'Peer (mDNS)', kind: 'P2P' },
  { name: 'Server-to-server relay', kind: 'P2P' },
  { name: 'Tauri-host filesystem', kind: 'Local' },
];

const kindColors: Record<string, string> = {
  Local: 'bg-info-bg text-info-foreground border-info',
  SSH: 'bg-success-bg text-success-foreground border-success',
  Cloud: 'bg-primary/10 text-primary border-primary/30',
  Network: 'bg-warning-bg text-warning-foreground border-warning',
  Legacy: 'bg-background-secondary text-foreground-secondary border-border',
  HTTP: 'bg-background-secondary text-foreground-secondary border-border',
  P2P: 'bg-accent/10 text-accent border-accent/30',
};

export function ConnectorGrid() {
  return (
    <section id="connectors" className="marketing-section bg-background-secondary">
      <div className="container-wide">
        <div className="text-center max-w-3xl mx-auto mb-12">
          <h2 className="marketing-heading">17 protocols. One app. Zero adapters.</h2>
          <p className="marketing-subheading mt-4">
            Every connector implements the same crash-safe transfer engine. Resume, checksum, throttle, and recover the same way no matter what's at the other end.
          </p>
        </div>

        <div className="grid grid-cols-2 md:grid-cols-3 lg:grid-cols-4 gap-3">
          {connectors.map((c) => (
            <div
              key={c.name}
              className="bg-background-elevated rounded-lg border border-border p-4 flex items-center justify-between"
            >
              <span className="font-medium text-sm text-foreground">{c.name}</span>
              <span className={`text-xs px-2 py-0.5 rounded-full border ${kindColors[c.kind] ?? ''}`}>{c.kind}</span>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}
