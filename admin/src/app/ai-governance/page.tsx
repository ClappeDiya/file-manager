'use client';

import React, { useEffect, useState } from 'react';
import { Brain, Shield, Eye, EyeOff, Tag, FileText, Server, Sparkles, MessageSquare } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { StatCard } from '@/components/ui/stat-card';
import { Tabs, TabContent } from '@/components/ui/tabs';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useToastStore } from '@/lib/stores/toast-store';
import { useAuthStore } from '@/lib/stores/auth-store';
import { formatDateTime } from '@/lib/utils/format';
import type { AiFeature, AiGovernanceSettings, DataResidency } from '@/lib/db/repos/ai-governance';
import type { AuditRecord } from '@/lib/db/repos/audit';

const FEATURES: { id: AiFeature; name: string; description: string; icon: React.ComponentType<{ size?: number; className?: string }> }[] = [
  { id: 'classification', name: 'File Classification', description: 'AI-powered automatic file categorization and tagging', icon: Tag },
  { id: 'content_analysis', name: 'Content Analysis', description: 'Deep content analysis for compliance and security scanning', icon: Eye },
  { id: 'naming', name: 'Naming Suggestions', description: 'AI-suggested file and folder naming based on content and conventions', icon: FileText },
  { id: 'anomaly', name: 'Anomaly Detection', description: 'Detect unusual file access patterns and potential security threats', icon: Shield },
  { id: 'auto_suggestions', name: 'Automation Suggestions', description: 'Suggest automation rules from repeated file operations', icon: Sparkles },
  { id: 'natural_language_commands', name: 'Natural-language Commands', description: 'Turn typed requests into file operations and rules', icon: MessageSquare },
];

type Settings = AiGovernanceSettings & { updatedAt: string | null; updatedBy: string };

export default function AIGovernancePage() {
  const notify = useToastStore((s) => s.notify);
  const canWrite = useAuthStore((s) => s.permissions?.includes('ai_governance.write') ?? false);
  const { data, error, reload } = useApi<{ settings: Settings }>('/api/ai-governance');
  const auditLog = useApi<{ entries: AuditRecord[]; total: number }>('/api/ai-governance?view=audit&limit=100');
  const [tab, setTab] = useState('settings');
  const [residency, setResidency] = useState<DataResidency>('local');
  const [excluded, setExcluded] = useState('');
  const [saving, setSaving] = useState(false);

  const settings = data?.settings;
  useEffect(() => {
    if (!settings) return;
    setResidency(settings.dataResidency);
    setExcluded(settings.excludedPaths.join(', '));
  }, [settings]);

  const save = async (patch: Partial<AiGovernanceSettings>, message: string) => {
    setSaving(true);
    try {
      await api('/api/ai-governance', { body: patch });
      await Promise.all([reload(), auditLog.reload()]);
      notify(message, 'success');
    } catch (e) {
      notify(errorText(e), 'error');
    } finally {
      setSaving(false);
    }
  };

  if (error) {
    return (
      <div>
        <PageHeader title="AI Governance" description="Control how AI features process organizational data with full transparency" />
        <p className="text-sm text-error mt-4">Could not load AI governance settings: {error}</p>
      </div>
    );
  }
  if (!settings) {
    return (
      <div>
        <PageHeader title="AI Governance" description="Control how AI features process organizational data with full transparency" />
        <p className="text-sm text-foreground-secondary mt-4">Loading…</p>
      </div>
    );
  }

  const enabledCount = FEATURES.filter((f) => settings.features[f.id]).length;
  const paths = excluded.split(',').map((p) => p.trim()).filter(Boolean);

  return (
    <div>
      <PageHeader
        title="AI Governance"
        description="Control how AI features process organizational data with full transparency"
        actions={
          canWrite ? (
            <Button
              size="sm"
              variant={settings.aiEnabled ? 'outline' : 'primary'}
              disabled={saving}
              onClick={() => save({ aiEnabled: !settings.aiEnabled }, settings.aiEnabled ? 'AI features turned off' : 'AI features turned on')}
            >
              {settings.aiEnabled ? 'Turn off all AI' : 'Turn on AI'}
            </Button>
          ) : undefined
        }
      />

      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard title="AI Features" value={settings.aiEnabled ? `${enabledCount}/${FEATURES.length}` : 'Off'} subtitle={settings.aiEnabled ? 'Enabled' : 'Turned off for this organization'} icon={Brain} variant="info" />
        <StatCard title="Data Residency" value={settings.dataResidency === 'local' ? 'Local Only' : settings.dataResidency === 'cloud' ? 'Cloud' : 'Hybrid'} icon={Server} variant="success" />
        <StatCard title="Excluded Paths" value={settings.excludedPaths.length} subtitle="Protected directories" icon={EyeOff} variant="default" />
      </div>

      <Tabs
        tabs={[
          { id: 'settings', label: 'Settings' },
          { id: 'audit', label: 'AI audit', count: auditLog.data?.total ?? 0 },
        ]}
        activeTab={tab}
        onTabChange={setTab}
        className="mb-4"
      />

      <TabContent tabId="settings" activeTab={tab}>
        <Card className="mb-6">
          <CardHeader>
            <CardTitle>Global AI Settings</CardTitle>
          </CardHeader>
          <CardContent>
            <div className="grid gap-4 max-w-xl">
              <Select
                label="Data Residency"
                options={[
                  { value: 'local', label: 'Local Only - All AI processing on device' },
                  { value: 'cloud', label: 'Cloud - Allow cloud AI processing' },
                  { value: 'hybrid', label: 'Hybrid - Local preferred, cloud fallback' },
                ]}
                value={residency}
                disabled={!canWrite}
                onChange={(e) => setResidency(e.target.value as DataResidency)}
              />
              <Input
                label="Excluded Paths"
                placeholder="/confidential, /legal"
                value={excluded}
                disabled={!canWrite}
                onChange={(e) => setExcluded(e.target.value)}
                hint="Comma-separated paths where AI features are completely disabled"
              />
              {canWrite && (
                <Button className="w-fit" disabled={saving} onClick={() => save({ dataResidency: residency, excludedPaths: paths }, 'AI settings saved')}>
                  Save Settings
                </Button>
              )}
              {settings.updatedAt && (
                <p className="text-xs text-foreground-tertiary">
                  Last changed {formatDateTime(settings.updatedAt)} by {settings.updatedBy}
                </p>
              )}
            </div>
          </CardContent>
        </Card>

        <h2 className="text-lg font-semibold text-foreground mb-4">AI Features</h2>
        <div className="grid gap-4">
          {FEATURES.map((feature) => {
            const enabled = settings.aiEnabled && settings.features[feature.id];
            const Icon = feature.icon;
            return (
              <Card key={feature.id}>
                <div className="flex items-start gap-4">
                  <div className={`p-3 rounded-lg ${enabled ? 'bg-primary/10' : 'bg-background-tertiary'}`}>
                    <Icon size={20} className={enabled ? 'text-primary' : 'text-foreground-disabled'} />
                  </div>
                  <div className="flex-1">
                    <div className="flex items-center gap-2">
                      <h3 className="text-sm font-semibold text-foreground">{feature.name}</h3>
                      <Badge variant={enabled ? 'success' : 'default'}>{enabled ? 'Enabled' : 'Disabled'}</Badge>
                      <Badge variant="outline">{settings.dataResidency === 'local' ? 'Local Processing' : settings.dataResidency}</Badge>
                    </div>
                    <p className="text-xs text-foreground-secondary mt-1">{feature.description}</p>
                  </div>
                  {canWrite && (
                    <Button
                      variant={settings.features[feature.id] ? 'outline' : 'primary'}
                      size="sm"
                      disabled={saving || !settings.aiEnabled}
                      onClick={() =>
                        save(
                          { features: { ...settings.features, [feature.id]: !settings.features[feature.id] } },
                          `${feature.name} ${settings.features[feature.id] ? 'disabled' : 'enabled'}`,
                        )
                      }
                    >
                      {settings.features[feature.id] ? <><EyeOff size={14} /> Disable</> : <><Eye size={14} /> Enable</>}
                    </Button>
                  )}
                </div>
              </Card>
            );
          })}
        </div>
      </TabContent>

      <TabContent tabId="audit" activeTab={tab}>
        <Card>
          <CardContent>
            {!auditLog.data || auditLog.data.entries.length === 0 ? (
              <p className="py-8 text-center text-sm text-foreground-tertiary">No AI governance changes recorded yet.</p>
            ) : (
              <ul className="divide-y divide-border" aria-label="AI audit entries">
                {auditLog.data.entries.map((e) => (
                  <li key={e.id} className="py-3 text-sm">
                    <div className="flex items-center justify-between gap-2">
                      <span className="font-medium text-foreground">{e.description}</span>
                      <span className="text-xs text-foreground-tertiary shrink-0">{formatDateTime(e.timestamp)}</span>
                    </div>
                    <div className="text-xs text-foreground-secondary mt-1">
                      {e.userEmail || 'system'} · <span className="font-mono">{e.eventType}</span> · #{e.sequence}
                    </div>
                  </li>
                ))}
              </ul>
            )}
          </CardContent>
        </Card>
      </TabContent>
    </div>
  );
}
