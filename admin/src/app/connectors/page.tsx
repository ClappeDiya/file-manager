import Link from 'next/link';
import { PageHeader } from '@/components/layout/page-header';
import { ComingSoon } from '@/components/ui/coming-soon';

export default function ConnectorsPage() {
  return <div>
    <PageHeader title="Connectors" description="Manage storage connectors and integrations" />
    <ComingSoon feature="connectors" description="Organization-wide connector definitions and endpoint testing are not available in this console. Manage encrypted credentials for enrolled machines in Fleet. Connect to storage directly from the desktop app."
      action={<Link href="/fleet/credentials" className="text-sm font-medium text-primary hover:underline">Open Fleet credentials →</Link>} />
  </div>;
}
