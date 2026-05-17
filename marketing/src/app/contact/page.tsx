import type { Metadata } from 'next';
import { Mail, MessageCircle, Github, Shield } from 'lucide-react';

export const metadata: Metadata = {
  title: 'Contact',
  description: 'Get in touch with the FileManager team.',
};

const channels = [
  {
    icon: Mail,
    title: 'General inquiries',
    body: 'Product questions, partnership ideas, press, anything else.',
    cta: { label: 'hello@clappe.com', href: 'mailto:hello@clappe.com' },
  },
  {
    icon: MessageCircle,
    title: 'Sales',
    body: 'Enterprise pricing, procurement, security reviews, custom connectors.',
    cta: { label: 'sales@clappe.com', href: 'mailto:sales@clappe.com' },
  },
  {
    icon: Shield,
    title: 'Security',
    body: 'Responsible disclosure for vulnerabilities. Please do not file public issues.',
    cta: { label: 'security@clappe.com', href: 'mailto:security@clappe.com' },
  },
  {
    icon: Github,
    title: 'Bugs & feature requests',
    body: 'Open an issue on GitHub. We triage weekly and tag everything publicly.',
    cta: { label: 'github.com/ClappeDiya/file-manager', href: 'https://github.com/ClappeDiya/file-manager/issues' },
  },
];

export default function ContactPage() {
  return (
    <div className="container-page py-16">
      <header className="max-w-3xl mx-auto text-center mb-12">
        <h1 className="marketing-heading">Contact</h1>
        <p className="marketing-subheading mt-4">
          Real humans read every email. Response time is typically under one business day.
        </p>
      </header>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-6 max-w-4xl mx-auto">
        {channels.map((c) => (
          <div key={c.title} className="marketing-card">
            <c.icon size={24} className="text-primary mb-3" />
            <h2 className="font-semibold text-foreground mb-2">{c.title}</h2>
            <p className="text-sm text-foreground-secondary mb-3">{c.body}</p>
            <a href={c.cta.href} className="text-sm font-medium" rel="noopener">
              {c.cta.label}
            </a>
          </div>
        ))}
      </div>
    </div>
  );
}
