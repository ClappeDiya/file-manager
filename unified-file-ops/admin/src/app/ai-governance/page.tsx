'use client';

import React, { useState } from 'react';
import { Brain, Shield, Eye, EyeOff, Tag, FileText, Globe, Server } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardContent, CardHeader, CardTitle } from '@/components/ui/card';
import { Badge } from '@/components/ui/badge';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { StatCard } from '@/components/ui/stat-card';

interface AIFeatureConfig {
  id: string;
  name: string;
  description: string;
  enabled: boolean;
  iconName: string;
  dataResidency: 'local' | 'cloud' | 'hybrid';
}

const iconMap: Record<string, React.ComponentType<{ size?: number; className?: string }>> = {
  tag: Tag,
  eye: Eye,
  filetext: FileText,
  shield: Shield,
};

export default function AIGovernancePage() {
  const [features, setFeatures] = useState<AIFeatureConfig[]>([
    { id: 'classification', name: 'File Classification', description: 'AI-powered automatic file categorization and tagging', enabled: true, iconName: 'tag', dataResidency: 'local' },
    { id: 'content_analysis', name: 'Content Analysis', description: 'Deep content analysis for compliance and security scanning', enabled: false, iconName: 'eye', dataResidency: 'local' },
    { id: 'naming', name: 'Naming Suggestions', description: 'AI-suggested file and folder naming based on content and conventions', enabled: true, iconName: 'filetext', dataResidency: 'local' },
    { id: 'anomaly', name: 'Anomaly Detection', description: 'Detect unusual file access patterns and potential security threats', enabled: true, iconName: 'shield', dataResidency: 'local' },
  ]);

  const [excludedPaths, setExcludedPaths] = useState('/confidential, /legal, /hr');
  const [dataResidency, setDataResidency] = useState('local');

  const toggleFeature = (id: string) => {
    setFeatures(prev => prev.map(f => f.id === id ? { ...f, enabled: !f.enabled } : f));
  };

  const enabledCount = features.filter(f => f.enabled).length;

  return (
    <div>
      <PageHeader
        title="AI Governance"
        description="Control how AI features process organizational data with full transparency"
      />

      {/* Stats */}
      <div className="grid grid-cols-1 sm:grid-cols-3 gap-4 mb-6">
        <StatCard title="AI Features" value={`${enabledCount}/${features.length}`} subtitle="Enabled" icon={Brain} variant="info" />
        <StatCard title="Data Residency" value={dataResidency === 'local' ? 'Local Only' : dataResidency === 'cloud' ? 'Cloud' : 'Hybrid'} icon={Server} variant="success" />
        <StatCard title="Excluded Paths" value={excludedPaths.split(',').length} subtitle="Protected directories" icon={EyeOff} variant="default" />
      </div>

      {/* Global Settings */}
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
              value={dataResidency}
              onChange={(e) => setDataResidency(e.target.value)}
            />
            <Input
              label="Excluded Paths"
              placeholder="/confidential, /legal"
              value={excludedPaths}
              onChange={(e) => setExcludedPaths(e.target.value)}
              hint="Comma-separated paths where AI features are completely disabled"
            />
            <Button className="w-fit">Save Settings</Button>
          </div>
        </CardContent>
      </Card>

      {/* Feature Controls */}
      <h2 className="text-lg font-semibold text-foreground mb-4">AI Features</h2>
      <div className="grid gap-4">
        {features.map((feature) => {
          const Icon = iconMap[feature.iconName] || Shield;
          return (
            <Card key={feature.id}>
              <div className="flex items-start gap-4">
                <div className={`p-3 rounded-lg ${feature.enabled ? 'bg-primary/10' : 'bg-background-tertiary'}`}>
                  <Icon size={20} className={feature.enabled ? 'text-primary' : 'text-foreground-disabled'} />
                </div>
                <div className="flex-1">
                  <div className="flex items-center gap-2">
                    <h3 className="text-sm font-semibold text-foreground">{feature.name}</h3>
                    <Badge variant={feature.enabled ? 'success' : 'default'}>
                      {feature.enabled ? 'Enabled' : 'Disabled'}
                    </Badge>
                    <Badge variant="outline">
                      {feature.dataResidency === 'local' ? 'Local Processing' : feature.dataResidency}
                    </Badge>
                  </div>
                  <p className="text-xs text-foreground-secondary mt-1">{feature.description}</p>
                </div>
                <Button
                  variant={feature.enabled ? 'outline' : 'primary'}
                  size="sm"
                  onClick={() => toggleFeature(feature.id)}
                >
                  {feature.enabled ? (
                    <><EyeOff size={14} /> Disable</>
                  ) : (
                    <><Eye size={14} /> Enable</>
                  )}
                </Button>
              </div>
            </Card>
          );
        })}
      </div>
    </div>
  );
}
