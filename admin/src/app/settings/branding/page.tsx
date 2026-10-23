'use client';

/**
 * Settings → Branding: the organization's name, color and logo in the
 * console, and the look and wording of its email (with a live preview and a
 * test send). Child organizations inherit until they set their own.
 */
import React, { useEffect, useState } from 'react';
import { PageHeader } from '@/components/layout/page-header';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { useToastStore } from '@/lib/stores/toast-store';
import { useBrandingStore } from '@/lib/stores/branding-store';
import { brandTokens } from '@/lib/brand-colors';
import type { Branding, EffectiveBranding, EmailTemplateKey } from '@/lib/branding';
import type { Channel } from '@/lib/db/repos/channels';

interface TemplateDef {
  label: string;
  vars: readonly string[];
  subject: string;
  body: string;
}

interface Data {
  branding: EffectiveBranding;
  own: (Branding & { logo: string | null; updatedBy: string; updatedAt: string }) | null;
  templates: Record<EmailTemplateKey, TemplateDef>;
  canEdit: boolean;
}

type Draft = Omit<Branding, 'templates'> & { logo: string | null; templates: Branding['templates'] };

const MAX_LOGO = 256 * 1024;

function toDraft(d: Data): Draft {
  const src = d.own ?? d.branding;
  return {
    displayName: src.displayName,
    primaryColor: src.primaryColor,
    logo: src.logo,
    emailFromName: src.emailFromName,
    emailFooter: src.emailFooter,
    supportEmail: src.supportEmail,
    supportUrl: src.supportUrl,
    templates: { ...src.templates },
  };
}

export default function BrandingPage() {
  const { data, error, loading, reload } = useApi<Data>('/api/branding');
  const channels = useApi<{ channels: Channel[] }>('/api/alerts/channels');
  const notify = useToastStore((s) => s.notify);
  const setBrand = useBrandingStore((s) => s.set);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [seeded, setSeeded] = useState<string | null>(null);
  const [tpl, setTpl] = useState<EmailTemplateKey>('invitation');
  const [preview, setPreview] = useState<{ subject: string; text: string; html: string } | null>(null);
  const [testChannel, setTestChannel] = useState('');
  const [err, setErr] = useState<string | null>(null);

  const stamp = data ? `${data.own?.updatedAt ?? ''}|${data.branding.sourceOrgId ?? ''}` : null;
  if (data && seeded !== stamp) {
    setSeeded(stamp);
    setDraft(toDraft(data));
  }

  // Live preview of the selected template (saved branding, draft wording).
  const current = draft?.templates[tpl];
  const defaults = data?.templates?.[tpl];
  const subject = current?.subject ?? defaults?.subject ?? '';
  const body = current?.body ?? defaults?.body ?? '';
  useEffect(() => {
    if (!data?.canEdit) return;
    const t = setTimeout(() => {
      api<{ subject: string; text: string; html: string }>('/api/branding/preview', { method: 'POST', body: { template: tpl, subject, body } })
        .then(setPreview)
        .catch((e) => setPreview({ subject: '', text: errorText(e), html: '' }));
    }, 300);
    return () => clearTimeout(t);
  }, [tpl, subject, body, data?.canEdit, data?.own?.updatedAt]);

  if (loading && !data) return <Loading />;
  if (!data || !draft) return <ErrorBanner message={error ?? 'Could not load the branding'} />;
  if (!data.canEdit) return <ErrorBanner message="You cannot change this organization's branding." />;
  const set = (patch: Partial<Draft>) => setDraft({ ...draft, ...patch });
  const setTemplate = (patch: Partial<{ subject: string; body: string }>) => set({ templates: { ...draft.templates, [tpl]: { subject, body, ...patch } } });
  const emailChannels = (channels.data?.channels ?? []).filter((c) => c.kind === 'email' && c.enabled);

  const save = async () => {
    setErr(null);
    try {
      const r = await api<{ branding: EffectiveBranding }>('/api/branding', { method: 'PUT', body: draft });
      setBrand(r.branding);
      notify('Branding saved', 'success');
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const resetAll = async () => {
    if (!confirm("Remove this organization's branding? It goes back to its parent's (or the standard look).")) return;
    try {
      const r = await api<{ branding: EffectiveBranding }>('/api/branding', { method: 'DELETE' });
      setBrand(r.branding);
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const pickLogo = (f: File | undefined) => {
    if (!f) return;
    if (!['image/png', 'image/jpeg', 'image/gif', 'image/webp'].includes(f.type)) return setErr('Use a PNG, JPEG, GIF or WebP image');
    if (f.size > MAX_LOGO) return setErr('The logo is limited to 256 KB');
    const reader = new FileReader();
    reader.onload = () => set({ logo: String(reader.result) });
    reader.readAsDataURL(f);
  };
  const sendTest = async () => {
    setErr(null);
    try {
      await api('/api/branding/test', { method: 'POST', body: { template: tpl, channelId: testChannel } });
      notify('Test email sent to you', 'success');
    } catch (e) {
      setErr(errorText(e));
    }
  };
  const swatch = brandTokens(draft.primaryColor);

  return (
    <div>
      <PageHeader title="Branding" description="How the console and its email look to your organization's members." />
      {err && <ErrorBanner message={err} />}
      {data.branding.inherited && !data.own && <p className="mb-3 text-sm text-foreground-secondary">This organization uses its parent organization's branding. Saving here gives it its own.</p>}
      <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Console</CardTitle>
            <CardDescription>Name, color and logo in the console for members of this organization.</CardDescription>
          </CardHeader>
          <div className="space-y-3">
            <Input label="Name" value={draft.displayName} maxLength={80} onChange={(e) => set({ displayName: e.target.value })} />
            <div className="flex items-end gap-2">
              <Input label="Brand color" placeholder="#2563eb" value={draft.primaryColor ?? ''} onChange={(e) => set({ primaryColor: e.target.value || null })} />
              <input aria-label="Pick brand color" type="color" className="h-9 w-12 rounded border border-border" value={draft.primaryColor ?? '#2563eb'} onChange={(e) => set({ primaryColor: e.target.value })} />
              {swatch && (
                <span className="rounded px-2 py-1 text-xs" style={{ background: swatch['--color-primary'], color: swatch['--color-primary-foreground'] }}>
                  Button
                </span>
              )}
            </div>
            <div className="space-y-1 text-sm">
              <span className="font-medium">Logo</span>
              <div className="flex items-center gap-3">
                {draft.logo ? <img src={draft.logo} alt="Logo" className="h-10 max-w-[10rem] rounded border border-border object-contain" /> : <span className="text-xs text-foreground-secondary">No logo</span>}
                <input aria-label="Logo file" type="file" accept="image/png,image/jpeg,image/gif,image/webp" onChange={(e) => pickLogo(e.target.files?.[0])} className="text-xs" />
                {draft.logo && (
                  <Button size="sm" variant="outline" onClick={() => set({ logo: null })}>
                    Remove
                  </Button>
                )}
              </div>
              <CardDescription>PNG, JPEG, GIF or WebP up to 256 KB; shown in the console and at the top of email.</CardDescription>
            </div>
            <Input label="Support email" type="email" value={draft.supportEmail} onChange={(e) => set({ supportEmail: e.target.value })} />
            <Input label="Support link" placeholder="https://help.example.com" value={draft.supportUrl} onChange={(e) => set({ supportUrl: e.target.value })} />
          </div>
        </Card>
        <Card>
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Email</CardTitle>
            <CardDescription>Applies to email channels: an HTML version in your colors with your logo, your footer and sender name.</CardDescription>
          </CardHeader>
          <div className="space-y-3">
            <Input label="Sender name" placeholder={draft.displayName} value={draft.emailFromName} onChange={(e) => set({ emailFromName: e.target.value })} hint="Used when a channel's From address has no name." />
            <label className="flex flex-col gap-1.5 text-sm font-medium">
              Footer
              <textarea
                aria-label="Footer"
                className="min-h-20 rounded-md border border-border bg-background p-2 text-sm font-normal"
                maxLength={1000}
                value={draft.emailFooter}
                onChange={(e) => set({ emailFooter: e.target.value })}
              />
            </label>
          </div>
        </Card>
        <Card className="lg:col-span-2">
          <CardHeader className="flex-col items-start gap-1">
            <CardTitle>Email templates</CardTitle>
            <CardDescription>Placeholders: {data.templates[tpl].vars.map((v) => `{{${v}}}`).join(' ')}</CardDescription>
          </CardHeader>
          <div className="grid grid-cols-1 gap-4 lg:grid-cols-2">
            <div className="space-y-3">
              <Select
                label="Template"
                value={tpl}
                onChange={(e) => setTpl(e.target.value as EmailTemplateKey)}
                options={(Object.keys(data.templates) as EmailTemplateKey[]).map((k) => ({ value: k, label: `${data.templates[k].label}${draft.templates[k] ? ' (customized)' : ''}` }))}
              />
              <Input label="Subject" value={subject} maxLength={200} onChange={(e) => setTemplate({ subject: e.target.value })} />
              <label className="flex flex-col gap-1.5 text-sm font-medium">
                Text
                <textarea
                  aria-label="Template text"
                  className="min-h-48 rounded-md border border-border bg-background p-2 font-mono text-xs font-normal"
                  value={body}
                  onChange={(e) => setTemplate({ body: e.target.value })}
                />
              </label>
              <div className="flex flex-wrap items-end gap-2">
                <Button
                  size="sm"
                  variant="outline"
                  disabled={!draft.templates[tpl]}
                  onClick={() => {
                    const { [tpl]: _gone, ...rest } = draft.templates;
                    set({ templates: rest });
                  }}
                >
                  Use the default
                </Button>
                {emailChannels.length > 0 && (
                  <>
                    <Select
                      label="Test through"
                      value={testChannel}
                      onChange={(e) => setTestChannel(e.target.value)}
                      options={[{ value: '', label: 'Choose an email channel' }, ...emailChannels.map((c) => ({ value: c.id, label: c.name }))]}
                    />
                    <Button size="sm" variant="outline" disabled={!testChannel} onClick={() => void sendTest()}>
                      Send me a test (saved version)
                    </Button>
                  </>
                )}
              </div>
            </div>
            <div className="space-y-2" aria-label="Email preview">
              <p className="text-sm">
                <span className="text-foreground-secondary">Subject:</span> {preview?.subject}
              </p>
              {preview?.html ? (
                <iframe title="Email preview" sandbox="" className="h-96 w-full rounded border border-border bg-white" srcDoc={preview.html} />
              ) : (
                <pre className="whitespace-pre-wrap rounded border border-border p-2 text-xs">{preview?.text}</pre>
              )}
              <CardDescription>The preview uses the saved colors and logo; save to see changes to them.</CardDescription>
            </div>
          </div>
        </Card>
      </div>
      <div className="mt-4 flex gap-2">
        <Button onClick={() => void save()}>Save</Button>
        {data.own && (
          <Button variant="outline" onClick={() => void resetAll()}>
            Remove this organization&apos;s branding
          </Button>
        )}
      </div>
    </div>
  );
}
