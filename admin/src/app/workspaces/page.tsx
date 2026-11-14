import Link from 'next/link';
import { PageHeader } from '@/components/layout/page-header';
import { ComingSoon } from '@/components/ui/coming-soon';

export default function WorkspacesPage() {
  return <div>
    <PageHeader title="Shared Workspaces" description="Shared connections, sync templates and team access" />
    <ComingSoon feature="sharedWorkspaces" description="Shared workspace grants and template delivery are not available yet. No access has been granted and no desktop configuration has been propagated from this screen. Use Fleet to manage enrolled machines and their configuration."
      action={<Link href="/fleet/credentials" className="text-sm font-medium text-primary hover:underline">Manage Fleet credentials →</Link>} />
  </div>;
}
