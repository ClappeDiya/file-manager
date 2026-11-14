import Link from 'next/link';

const capabilities = [
  ['Transfer files', 'Browse two locations and copy or move files between them. Follow progress and inspect individual results.'],
  ['Compare and sync folders', 'Preview folder differences and choose how changes should flow before applying them.'],
  ['Back up and restore', 'Keep versioned backups and restore selected files to a folder you choose.'],
  ['Connect storage', 'Open saved cloud and server connections alongside local folders. Availability depends on the provider and your credentials.'],
  ['Automate from the CLI', 'Use dry runs and structured output for repeatable local file operations.'],
  ['Manage a team', 'The Business admin console provides role-based access, policies, approvals and audit history.'],
];

/** Describe our workflows instead of publishing unsourced competitor scores. */
export function ComparisonTable() {
  return <section className="marketing-section" aria-labelledby="workflow-heading">
    <div className="container-page">
      <div className="max-w-3xl mb-8">
        <h2 id="workflow-heading" className="marketing-heading">Choose the workflow you need</h2>
        <p className="marketing-subheading mt-4">Start with files and folders. Open more tools when your task calls for them.</p>
      </div>
      <dl className="grid gap-6 md:grid-cols-2">
        {capabilities.map(([label, description]) => <div key={label}>
          <dt className="font-semibold text-foreground">{label}</dt>
          <dd className="mt-2 text-foreground-secondary">{description}</dd>
        </div>)}
      </dl>
      <Link href="/docs" className="inline-block mt-8 underline underline-offset-4">Read the guides and requirements</Link>
    </div>
  </section>;
}
