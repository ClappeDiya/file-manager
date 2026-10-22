'use client';

import React, { useState } from 'react';
import { Fingerprint, Plus } from 'lucide-react';
import { PageHeader } from '@/components/layout/page-header';
import { Card } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { EmptyState } from '@/components/ui/empty-state';
import { Dialog, DialogDescription, DialogFooter, DialogHeader, DialogTitle } from '@/components/ui/dialog';
import { ErrorBanner, Loading } from '@/components/fleet/badges';
import { useApi } from '@/lib/use-api';
import { api, errorText } from '@/lib/api-client';
import { ROLE_LABELS, type Role } from '@/lib/types/auth';
import type { OidcConfig, SamlConfig, SsoKind, SsoProvider } from '@/lib/db/repos/sso';
import type { Group } from '@/lib/db/repos/groups';
import { SsoDiagnostics } from '@/components/users/sso-diagnostics';

type Row = SsoProvider & { callbackUrl: string; metadataUrl: string | null };

interface Draft {
  id: string | null;
  name: string;
  kind: SsoKind;
  oidc: OidcConfig;
  saml: SamlConfig;
  secret: string;
  hasSecret: boolean;
  defaultRole: Role;
  defaultGroupId: string;
  mapping: string;
  requireMfa: boolean;
  allowJit: boolean;
  domains: string;
  enabled: boolean;
}

const JIT_ROLES: Role[] = ['user', 'read_only_auditor', 'operator', 'org_admin'];
const emptyOidc: OidcConfig = { issuer: '', clientId: '', scopes: 'openid email profile', allowUnverifiedEmail: false, groupsClaim: 'groups' };
const emptySaml: SamlConfig = { entryPoint: '', idpCert: '', idpIssuer: '', emailAttribute: '', nameAttribute: 'displayName', groupsAttribute: 'groups' };

/** "idp-group = console group" lines ↔ mapping object. */
const toLines = (m: Record<string, string>, groups: Group[]) => Object.entries(m).map(([k, v]) => `${k} = ${groups.find((g) => g.id === v)?.name ?? v}`).join('\n');
function fromLines(s: string, groups: Group[]): Record<string, string> {
  const out: Record<string, string> = {};
  for (const line of s.split('\n')) {
    const [k, v] = line.split('=').map((x) => x?.trim());
    if (!k || !v) continue;
    out[k] = groups.find((g) => g.name.toLowerCase() === v.toLowerCase())?.id ?? v;
  }
  return out;
}

export default function SsoSettingsPage() {
  const { data, error, loading, reload } = useApi<{ providers: Row[] }>('/api/sso');
  const groups = useApi<{ groups: Group[] }>('/api/groups');
  const [draft, setDraft] = useState<Draft | null>(null);
  const [formErr, setFormErr] = useState<string | null>(null);
  const [diagnose, setDiagnose] = useState<SsoProvider | null>(null);
  const [err, setErr] = useState<string | null>(null);
  const gs = groups.data?.groups ?? [];

  const edit = (p: Row | null) => {
    setFormErr(null);
    setDraft(
      p
        ? { id: p.id, name: p.name, kind: p.kind, oidc: p.kind === 'oidc' ? { ...emptyOidc, ...(p.config as OidcConfig) } : emptyOidc, saml: p.kind === 'saml' ? { ...emptySaml, ...(p.config as SamlConfig) } : emptySaml, secret: '', hasSecret: p.hasSecret, defaultRole: p.defaultRole, defaultGroupId: p.defaultGroupId ?? '', mapping: toLines(p.groupMapping, gs), requireMfa: p.requireMfa, allowJit: p.allowJit, domains: p.domains.join(', '), enabled: p.enabled }
        : { id: null, name: '', kind: 'oidc', oidc: emptyOidc, saml: emptySaml, secret: '', hasSecret: false, defaultRole: 'user', defaultGroupId: '', mapping: '', requireMfa: false, allowJit: true, domains: '', enabled: true },
    );
  };

  const save = async () => {
    if (!draft) return;
    setFormErr(null);
    const body: Record<string, unknown> = {
      name: draft.name,
      kind: draft.kind,
      config: draft.kind === 'oidc' ? draft.oidc : draft.saml,
      defaultRole: draft.defaultRole,
      defaultGroupId: draft.defaultGroupId || null,
      groupMapping: fromLines(draft.mapping, gs),
      requireMfa: draft.requireMfa,
      allowJit: draft.allowJit,
      domains: draft.domains.split(',').map((d) => d.trim()).filter(Boolean),
      enabled: draft.enabled,
    };
    if (draft.secret) body.secret = draft.secret;
    try {
      await api(draft.id ? `/api/sso/${draft.id}` : '/api/sso', { method: draft.id ? 'PATCH' : 'POST', body });
      setDraft(null);
      await reload();
    } catch (e) {
      setFormErr(errorText(e));
    }
  };

  const remove = async (p: Row) => {
    if (!confirm(`Remove "${p.name}"? People who only sign in with it will need another method.`)) return;
    try {
      await api(`/api/sso/${p.id}`, { method: 'DELETE' });
      await reload();
    } catch (e) {
      setErr(errorText(e));
    }
  };

  return (
    <div>
      <PageHeader
        title="Single sign-on"
        breadcrumbs={[{ label: 'Dashboard', href: '/dashboard' }, { label: 'Single sign-on' }]}
        description="Let people sign in with your identity provider (OpenID Connect or SAML 2.0)."
        actions={
          <Button onClick={() => edit(null)}>
            <Plus size={14} className="mr-2" aria-hidden="true" />
            Add provider
          </Button>
        }
      />
      <ErrorBanner message={err ?? error} />
      {loading && !data ? (
        <Loading />
      ) : !data?.providers.length ? (
        <Card>
          <EmptyState icon={Fingerprint} title="No identity providers" description="Add Okta, Entra ID, Google Workspace, Keycloak or any OIDC / SAML provider." />
        </Card>
      ) : (
        <div className="space-y-3">
          {data.providers.map((p) => (
            <Card key={p.id}>
              <div className="flex flex-col gap-3 md:flex-row md:items-start md:justify-between">
                <div className="min-w-0">
                  <p className="font-semibold">
                    {p.name} <span className="text-xs font-normal text-foreground-tertiary">{p.kind.toUpperCase()}</span>
                    {!p.enabled && <span className="ml-2 text-xs text-warning">disabled</span>}
                  </p>
                  <p className="text-xs text-foreground-secondary">
                    Domains: {p.domains.join(', ') || 'any (no automatic account linking)'} · new accounts: {p.allowJit ? ROLE_LABELS[p.defaultRole] : 'invitation only'}
                    {p.requireMfa ? ' · requires two-factor' : ''}
                  </p>
                  <p className="mt-2 text-xs text-foreground-tertiary">
                    {p.kind === 'oidc' ? 'Redirect URI' : 'ACS URL'}: <code className="break-all">{p.callbackUrl}</code>
                  </p>
                  {p.metadataUrl && (
                    <p className="text-xs text-foreground-tertiary">
                      SP metadata:{' '}
                      <a className="break-all text-primary" href={p.metadataUrl} target="_blank" rel="noreferrer">
                        {p.metadataUrl}
                      </a>
                    </p>
                  )}
                </div>
                <div className="flex shrink-0 gap-2">
                  <Button size="sm" variant="outline" onClick={() => setDiagnose(p)}>
                    Diagnostics
                  </Button>
                  <Button size="sm" variant="outline" onClick={() => edit(p)}>
                    Edit
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => void remove(p)}>
                    Remove
                  </Button>
                </div>
              </div>
            </Card>
          ))}
        </div>
      )}

      <Dialog open={!!draft} onClose={() => setDraft(null)} className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>{draft?.id ? `Edit "${draft.name}"` : 'Add identity provider'}</DialogTitle>
          <DialogDescription>After saving, register the redirect (OIDC) or ACS URL (SAML) shown on the provider card with your identity provider.</DialogDescription>
        </DialogHeader>
        {draft && (
          <div className="max-h-[60vh] space-y-3 overflow-y-auto pr-1">
            <ErrorBanner message={formErr} />
            <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
              <Input label="Name shown on the sign-in page" value={draft.name} onChange={(e) => setDraft({ ...draft, name: e.target.value })} />
              <Select label="Protocol" value={draft.kind} disabled={!!draft.id} onChange={(e) => setDraft({ ...draft, kind: e.target.value as SsoKind })} options={[{ value: 'oidc', label: 'OpenID Connect' }, { value: 'saml', label: 'SAML 2.0' }]} />
            </div>
            {draft.kind === 'oidc' ? (
              <>
                <Input label="Issuer URL" placeholder="https://login.example.com" value={draft.oidc.issuer} onChange={(e) => setDraft({ ...draft, oidc: { ...draft.oidc, issuer: e.target.value } })} />
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <Input label="Client ID" value={draft.oidc.clientId} onChange={(e) => setDraft({ ...draft, oidc: { ...draft.oidc, clientId: e.target.value } })} />
                  <Input label="Client secret" type="password" autoComplete="new-password" placeholder={draft.hasSecret ? '•••••• (unchanged)' : ''} value={draft.secret} onChange={(e) => setDraft({ ...draft, secret: e.target.value })} />
                </div>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                  <Input label="Scopes" value={draft.oidc.scopes} onChange={(e) => setDraft({ ...draft, oidc: { ...draft.oidc, scopes: e.target.value } })} />
                  <Input label="Groups claim" value={draft.oidc.groupsClaim ?? ''} onChange={(e) => setDraft({ ...draft, oidc: { ...draft.oidc, groupsClaim: e.target.value } })} />
                </div>
                <label className="flex items-center gap-2 text-sm">
                  <input type="checkbox" checked={!!draft.oidc.allowUnverifiedEmail} onChange={(e) => setDraft({ ...draft, oidc: { ...draft.oidc, allowUnverifiedEmail: e.target.checked } })} />
                  Accept email addresses the provider has not verified
                </label>
              </>
            ) : (
              <>
                <Input label="IdP sign-on URL" placeholder="https://idp.example.com/sso/saml" value={draft.saml.entryPoint} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, entryPoint: e.target.value } })} />
                <Input label="IdP entity ID (issuer)" value={draft.saml.idpIssuer ?? ''} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, idpIssuer: e.target.value } })} />
                <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
                  IdP signing certificate (PEM)
                  <textarea className="min-h-28 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.saml.idpCert} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, idpCert: e.target.value } })} spellCheck={false} />
                </label>
                <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
                  <Input label="Email attribute" placeholder="NameID" value={draft.saml.emailAttribute ?? ''} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, emailAttribute: e.target.value } })} />
                  <Input label="Name attribute" value={draft.saml.nameAttribute ?? ''} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, nameAttribute: e.target.value } })} />
                  <Input label="Groups attribute" value={draft.saml.groupsAttribute ?? ''} onChange={(e) => setDraft({ ...draft, saml: { ...draft.saml, groupsAttribute: e.target.value } })} />
                </div>
              </>
            )}
            <Input label="Email domains (comma separated)" placeholder="example.com" value={draft.domains} onChange={(e) => setDraft({ ...draft, domains: e.target.value }) } hint="Shown on the sign-in page for these domains; existing accounts in them are linked on first sign-in." />
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={draft.allowJit} onChange={(e) => setDraft({ ...draft, allowJit: e.target.checked })} />
              Create accounts on first sign-in
            </label>
            {draft.allowJit && (
              <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
                <Select label="Role for new accounts" value={draft.defaultRole} onChange={(e) => setDraft({ ...draft, defaultRole: e.target.value as Role })} options={JIT_ROLES.map((r) => ({ value: r, label: ROLE_LABELS[r] }))} />
                <Select label="Default group" value={draft.defaultGroupId} onChange={(e) => setDraft({ ...draft, defaultGroupId: e.target.value })} placeholder="None" options={gs.map((g) => ({ value: g.id, label: g.name }))} />
              </div>
            )}
            <label className="flex flex-col gap-1.5 text-sm font-medium text-foreground">
              Group mapping on first sign-in (one per line: idp-group = console group)
              <textarea className="min-h-20 rounded-md border border-border bg-background p-2 font-mono text-xs" value={draft.mapping} onChange={(e) => setDraft({ ...draft, mapping: e.target.value })} placeholder={'engineering = Engineers\nit-admins = Backup operators'} spellCheck={false} />
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={draft.requireMfa} onChange={(e) => setDraft({ ...draft, requireMfa: e.target.checked })} />
              Also require the console&apos;s two-factor code
            </label>
            <label className="flex items-center gap-2 text-sm">
              <input type="checkbox" checked={draft.enabled} onChange={(e) => setDraft({ ...draft, enabled: e.target.checked })} />
              Enabled
            </label>
          </div>
        )}
        <DialogFooter>
          <Button variant="outline" onClick={() => setDraft(null)}>
            Cancel
          </Button>
          <Button onClick={() => void save()}>Save</Button>
        </DialogFooter>
      </Dialog>
      <SsoDiagnostics provider={diagnose} onClose={() => setDiagnose(null)} />
    </div>
  );
}
