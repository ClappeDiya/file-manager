import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Privacy Policy',
};

export default function PrivacyPage() {
  return (
    <div className="container-prose py-16 text-sm text-foreground-secondary leading-relaxed space-y-5">
      <h1 className="marketing-heading">Privacy Policy</h1>
      <p className="text-foreground-tertiary text-xs">Last updated: 2026-11-14</p>

      <p>
        This page describes how Clappe ("we") handles personal information when you use FileManager. It is written
        in plain language so you can actually read it.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">What the desktop app sends to us</h2>
      <p>
        Almost nothing. The FileManager desktop application is a local program. It does not phone home with usage
        data. It periodically checks our update server for a new version manifest; this request reveals only your
        IP and the current app version — no identifiers tied to your installation, no list of your files, no
        information about which connectors you use.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">What our website collects</h2>
      <p>
        Our website uses server logs for operation and abuse prevention. These can include your IP address,
        browser information, requested page and response status. The production website also uses first-party
        Matomo analytics, served from /mtm/, to measure page visits and link clicks. Where enabled, Matomo
        cookies can recognize returning browsers and group visits into sessions. Website measurement is
        separate from the desktop application's settings.
      </p>
      <p>
        If you submit the waitlist form, we use the details you provide to contact you about Business availability.
        Contact privacy@clappe.com to request access or deletion, or to ask about the current retention settings
        for website logs, analytics and waitlist records.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Your credentials</h2>
      <p>
        Cloud-storage API tokens, SSH keys, and OAuth refresh tokens you configure in the desktop app are stored
        in your operating system's keychain. They never reach our servers and never appear in our logs. If our
        infrastructure were fully compromised tomorrow, your credentials would not be exposed because we do not
        have them.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Business / Enterprise plans</h2>
      <p>
        Once Business launches, we store the information required to operate the subscription: org name, billing
        email, payment-method identifier issued by Stripe (never the card number itself), a list of seat
        assignments, and audit-log events your admin configured to forward to our cloud. Audit content is yours;
        you can export or delete it at any time.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Your rights</h2>
      <p>
        You may request a copy or deletion of any personal data we hold about you by emailing
        <a href="mailto:privacy@clappe.com"> privacy@clappe.com</a>. We respond within 30 days.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Changes</h2>
      <p>
        We post material changes here at least 14 days before they take effect, and email anyone on the Business
        plan directly. Minor wording changes are made without notice but the diff history of this page is public
        on our repo.
      </p>
    </div>
  );
}
