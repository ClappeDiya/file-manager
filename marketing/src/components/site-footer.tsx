import Link from 'next/link';

const sections = [
  {
    title: 'Product',
    links: [
      { href: '/download', label: 'Download' },
      { href: '/pricing', label: 'Pricing' },
      { href: '/changelog', label: 'Changelog' },
      { href: '/docs', label: 'Docs' },
    ],
  },
  {
    title: 'Company',
    links: [
      { href: 'https://clappe.com', label: 'Clappe' },
      { href: '/contact', label: 'Contact' },
      { href: '/security', label: 'Security' },
    ],
  },
  {
    title: 'Legal',
    links: [
      { href: '/legal/privacy', label: 'Privacy' },
      { href: '/legal/terms', label: 'Terms' },
      { href: '/legal/refund', label: 'Refunds' },
      { href: 'https://github.com/ClappeDiya/file-manager/blob/master/LICENSE', label: 'License' },
    ],
  },
];

export function SiteFooter() {
  return (
    <footer className="border-t border-border bg-background-secondary mt-24">
      <div className="container-wide py-12 grid grid-cols-2 md:grid-cols-4 gap-8">
        <div className="col-span-2 md:col-span-1">
          <div className="flex items-center gap-2 mb-3">
            <div className="w-7 h-7 rounded-md bg-primary text-primary-foreground grid place-items-center font-bold text-sm">F</div>
            <span className="font-semibold">FileManager</span>
          </div>
          <p className="text-sm text-foreground-secondary">
            The cross-platform file manager for power users. Local-first. Free for individuals.
          </p>
        </div>

        {sections.map((section) => (
          <div key={section.title}>
            <h3 className="text-sm font-semibold text-foreground mb-3">{section.title}</h3>
            <ul className="space-y-2">
              {section.links.map((link) => (
                <li key={link.href}>
                  <Link href={link.href} className="text-sm text-foreground-secondary hover:text-foreground transition-colors">
                    {link.label}
                  </Link>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>

      <div className="border-t border-border">
        <div className="container-wide py-6 flex flex-col md:flex-row items-center justify-between gap-3 text-xs text-foreground-tertiary">
          <p>© {new Date().getFullYear()} Clappe. Source-available under PolyForm Shield 1.0.0.</p>
          <p>Built locally. No telemetry by default.</p>
        </div>
      </div>
    </footer>
  );
}
