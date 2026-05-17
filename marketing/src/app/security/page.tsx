import type { Metadata } from 'next';
import { ShieldCheck, KeyRound, Cpu, Eye, Mail } from 'lucide-react';

export const metadata: Metadata = {
  title: 'Security & Privacy',
  description: 'Local-first by design. No telemetry by default. Your credentials never leave your OS keychain.',
};

const principles = [
  {
    icon: ShieldCheck,
    title: 'Local-first by design',
    body:
      "Every file operation runs on your machine. We do not relay your data through any cloud service, ever. When you connect to S3 or Google Drive, the connection is between your machine and that provider — we are not in the middle.",
  },
  {
    icon: KeyRound,
    title: 'Credentials stay in your OS keychain',
    body:
      'API tokens, SSH keys, and OAuth refresh tokens are stored in your platform keychain (Keychain on macOS, Credential Manager on Windows, libsecret on Linux). They never appear in our logs and never transit our infrastructure.',
  },
  {
    icon: Cpu,
    title: 'AI runs locally',
    body:
      "The AI assistant talks to your local Ollama install. We do not proxy AI requests through any hosted model. No API keys, no rate limits, no per-token billing — and no file contents leaving your machine.",
  },
  {
    icon: Eye,
    title: 'No telemetry by default',
    body:
      'The desktop app does not phone home. Auto-update checks fetch a static manifest; nothing about your usage is reported. If you opt into anonymous diagnostics later, you can audit exactly what is sent in Settings → Privacy.',
  },
];

export default function SecurityPage() {
  return (
    <div className="container-page py-16">
      <header className="max-w-3xl mx-auto text-center mb-12">
        <h1 className="marketing-heading">Security &amp; Privacy</h1>
        <p className="marketing-subheading mt-4">
          We built FileManager around a simple principle: a file manager should never need to see your files.
        </p>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6 mb-16">
        {principles.map((p) => (
          <div key={p.title} className="marketing-card">
            <p.icon size={24} className="text-primary mb-3" />
            <h2 className="font-semibold text-foreground mb-2">{p.title}</h2>
            <p className="text-sm text-foreground-secondary leading-relaxed">{p.body}</p>
          </div>
        ))}
      </div>

      <section className="marketing-card max-w-3xl mx-auto">
        <div className="flex items-center gap-3 mb-3">
          <Mail size={20} className="text-primary" />
          <h2 className="font-semibold text-foreground">Responsible disclosure</h2>
        </div>
        <p className="text-sm text-foreground-secondary mb-3">
          Found a security issue? Email <a href="mailto:security@clappe.com">security@clappe.com</a>. We aim to
          acknowledge within 24 hours and ship a patch within 14 days for high-severity issues. Please do not file
          public GitHub issues for security reports.
        </p>
        <p className="text-sm text-foreground-secondary">
          We currently do not operate a paid bug-bounty program but will publicly credit reporters (with permission)
          in the release notes for each fix.
        </p>
      </section>

      <section className="mt-16 max-w-3xl mx-auto text-sm text-foreground-secondary leading-relaxed space-y-4">
        <h2 className="text-xl font-bold text-foreground mb-3">Code signing &amp; updates</h2>
        <p>
          All macOS and Windows installers are code-signed with our Apple Developer ID and Windows EV certificate.
          The Tauri auto-updater verifies an Ed25519 signature on every update before applying it; a tampered
          update is rejected, not installed.
        </p>
        <p>
          Linux .AppImage and .deb / .rpm packages include detached signatures published alongside each release on
          GitHub. SHA-256 checksums are published for every artifact.
        </p>

        <h2 className="text-xl font-bold text-foreground mt-8 mb-3">Source-available</h2>
        <p>
          FileManager is published under the PolyForm Shield 1.0.0 license. You can read every line of the desktop
          code on GitHub and build it yourself. That means our claims above are auditable, not just marketing copy.
        </p>
      </section>
    </div>
  );
}
