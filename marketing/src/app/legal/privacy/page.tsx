import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Privacy Policy',
};

export default function PrivacyPage() {
  return (
    <div className="container-prose py-16 text-sm text-foreground-secondary leading-relaxed space-y-5">
      <h1 className="marketing-heading">Privacy Policy</h1>
      <p className="text-foreground-tertiary text-xs">Last updated: 2026-11-15</p>

      <p>
        This page describes how Clappe ("we") handles personal information when you use FileManager. It is written
        in plain language so you can actually read it.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">What the desktop app sends to us</h2>
      <p>
        Standalone file operations run on your computer. Update checks request a new version manifest and can
        reveal your IP address and app version; those checks do not include a file list or connector inventory.
        Optional network features communicate with the services you choose, such as cloud storage, an AI provider,
        or your organization's management console, using their configured endpoints and permissions.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">What our website collects</h2>
      <p>
        Our website uses server logs for operation and abuse prevention. These can include your IP address,
        browser information, requested page and response status. The production website also uses first-party
        Matomo analytics, served through /mtm/, to measure page visits and link clicks. The hosting proxy adds this
        measurement to website pages. Where enabled, analytics cookies can recognize returning browsers and group
        visits into sessions. Website measurement is separate from the desktop application's settings.
      </p>
      <p>
        If you submit the waitlist form, we use the details you provide to contact you about Business availability.
        Contact privacy@clappe.com to request access or deletion, or to ask about the current retention settings
        for website logs, analytics and waitlist records.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Your credentials</h2>
      <p>
        Credentials entered in a personal desktop connection are not automatically uploaded to our hosted
        console. The desktop uses the operating system's credential store or an encrypted portable vault for
        supported saved secrets.
      </p>
      <p>
        If your organization provisions a managed connector, its chosen admin console also stores an encrypted
        copy of that credential and delivers it to enrolled desktops that an organization administrator has
        explicitly granted access. Removing a grant removes the managed connection and delivered secret when
        the desktop next synchronizes successfully. The storage provider controls its own account permissions.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Managed consoles and subscriptions</h2>
      <p>
        When you use an admin console, its operator stores the account, organization, role, device enrollment,
        configuration and audit information needed to provide that service. Enrolled desktops can send machine
        identity, job status and diagnostic details allowed by their local management settings. Your administrator
        chooses enrollment and access. Audit exports and deletion depend on the console's permissions and retention
        policy.
      </p>
      <p>
        If a paid subscription is activated, billing also uses your organization's billing details and the
        payment-method identifier issued by Stripe, rather than your full card number.
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
