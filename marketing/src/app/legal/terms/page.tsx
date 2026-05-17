import type { Metadata } from 'next';

export const metadata: Metadata = {
  title: 'Terms of Service',
};

export default function TermsPage() {
  return (
    <div className="container-prose py-16 text-sm text-foreground-secondary leading-relaxed space-y-5">
      <h1 className="marketing-heading">Terms of Service</h1>
      <p className="text-foreground-tertiary text-xs">Last updated: {new Date().toISOString().slice(0, 10)}</p>

      <p>
        These terms govern your use of the FileManager desktop application and any paid subscription
        ("Service") provided by Clappe ("we").
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">License</h2>
      <p>
        The FileManager desktop application is licensed under PolyForm Shield 1.0.0. You may use, modify, and
        redistribute it for any purpose except building a competing product. The full license text is included
        with every download and is also available in the source repository.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Free plan</h2>
      <p>
        The desktop app is offered without warranty. We will do our best to keep it stable and update it
        regularly, but we make no SLA commitments for free users. If something breaks, please report it at
        <a href="https://github.com/ClappeDiya/file-manager/issues"> github.com/ClappeDiya/file-manager/issues</a>.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Paid plans</h2>
      <p>
        Business and Enterprise subscriptions are billed in advance per seat. Cancellation takes effect at the end
        of the current period; we do not pro-rate or refund partial periods except as required by law (see our
        refund policy). We retain the right to suspend service for non-payment after 14 days of grace.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Acceptable use</h2>
      <p>
        You agree not to use FileManager to violate any applicable law, infringe intellectual-property rights, or
        send malware. We reserve the right to terminate your subscription if we have a good-faith belief you are
        violating these terms — but we owe you the right to be heard first.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Liability</h2>
      <p>
        Our total liability for any claim arising from your use of the Service is limited to the amount you paid
        in the twelve months preceding the claim. We are not liable for indirect, incidental, or consequential
        damages.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Governing law</h2>
      <p>
        These terms are governed by the laws of the jurisdiction in which Clappe is incorporated, without regard
        to conflict-of-laws provisions. Disputes are subject to the exclusive jurisdiction of those courts.
      </p>

      <h2 className="text-xl font-bold text-foreground mt-8">Changes</h2>
      <p>
        We will notify Business and Enterprise customers of material changes at least 30 days in advance. Free
        users will see updated terms posted on this page.
      </p>

      <p className="text-xs text-foreground-tertiary mt-12">
        Note: these terms are a starting point. Run them past your own lawyer before relying on them in a
        commercial context.
      </p>
    </div>
  );
}
