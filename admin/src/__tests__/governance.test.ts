/**
 * AI governance is persisted per organization, permission-checked, validated
 * and audited on the hash chain; console features without a backend answer
 * 501 instead of keeping data in process memory.
 */
import { describe, expect, it } from 'vitest';
import { NextRequest } from 'next/server';
import { createOrg, setMembership } from '@/lib/db/repos/orgs';
import { listAudit, verifyAudit } from '@/lib/db/repos/audit';
import { getAiGovernance } from '@/lib/db/repos/ai-governance';
import { json, makeUser, req } from './helpers';
import * as gov from '@/app/api/ai-governance/route';
import * as webhooks from '@/app/api/webhooks/route';
import * as webhookById from '@/app/api/webhooks/[id]/route';
import * as sync from '@/app/api/sync/route';
import * as transfers from '@/app/api/transfers/route';
import * as connectors from '@/app/api/connectors/route';

const P = (id: string) => ({ params: Promise.resolve({ id }) });

describe('AI governance', () => {
  it('requires a real session (any bearer token used to pass)', async () => {
    const anon = new NextRequest(new URL('http://localhost:3001/api/ai-governance'), {
      headers: { authorization: 'Bearer anything', host: 'localhost:3001' },
    });
    expect((await gov.GET(anon)).status).toBe(401);
  });

  it('roles without ai_governance permissions are refused', async () => {
    const auditor = makeUser('auditor@x.org', 'read_only_auditor');
    expect((await gov.GET(await req('GET', '/api/ai-governance', { as: auditor }))).status).toBe(403);
    const res = await gov.POST(await req('POST', '/api/ai-governance', { as: auditor, body: { aiEnabled: false } }));
    expect(res.status).toBe(403);
  });

  it('persists, validates and audits changes', async () => {
    const admin = makeUser('sec@x.org', 'security_admin');
    const { body: initial } = await json<{ settings: { aiEnabled: boolean; dataResidency: string } }>(
      await gov.GET(await req('GET', '/api/ai-governance', { as: admin })),
    );
    expect(initial.settings).toMatchObject({ aiEnabled: true, dataResidency: 'local' });

    const saved = await json<{ settings: { features: Record<string, boolean>; excludedPaths: string[] } }>(
      await gov.POST(
        await req('POST', '/api/ai-governance', {
          as: admin,
          body: { dataResidency: 'hybrid', excludedPaths: [' /legal ', '/hr', '/legal'], features: { content_analysis: true } },
        }),
      ),
    );
    expect(saved.status).toBe(200);
    expect(saved.body.settings.excludedPaths).toEqual(['/legal', '/hr']);
    expect(saved.body.settings.features.content_analysis).toBe(true);

    // Stored in SQLite, not module memory.
    expect(getAiGovernance('org_default')).toMatchObject({ dataResidency: 'hybrid', updatedBy: 'sec@x.org' });

    for (const bad of [{ dataResidency: 'mars' }, { features: { telepathy: true } }, { maxFilesPerAction: 0 }, { allowedActions: ['format_disk'] }, { aiEnabled: 'yes' }]) {
      expect((await gov.POST(await req('POST', '/api/ai-governance', { as: admin, body: bad }))).status).toBe(400);
    }

    const entries = listAudit({ eventPrefix: 'ai.' }).entries;
    expect(entries).toHaveLength(1);
    expect(entries[0]).toMatchObject({ eventType: 'ai.governance.updated', severity: 'warning', userEmail: 'sec@x.org' });
    expect(entries[0].metadata.changed).toEqual(expect.arrayContaining(['dataResidency', 'excludedPaths', 'features']));
    expect(verifyAudit().ok).toBe(true);

    const { body: auditView } = await json<{ total: number }>(await gov.GET(await req('GET', '/api/ai-governance?view=audit', { as: admin })));
    expect(auditView.total).toBe(1);
  });

  it('accepts the legacy snake_case keys', async () => {
    const admin = makeUser('legacy@x.org', 'security_admin');
    const res = await json<{ settings: { aiEnabled: boolean; features: Record<string, boolean>; maxFilesPerAction: number } }>(
      await gov.POST(await req('POST', '/api/ai-governance', { as: admin, body: { ai_enabled: false, auto_suggestions: false, max_files_per_action: 25 } })),
    );
    expect(res.body.settings).toMatchObject({ aiEnabled: false, maxFilesPerAction: 25 });
    expect(res.body.settings.features.auto_suggestions).toBe(false);
  });

  it('is isolated per organization', async () => {
    const other = createOrg({ name: 'Other' });
    const admin = makeUser('two@x.org', 'security_admin');
    setMembership(admin.id, other.id, 'security_admin');
    await gov.POST(await req('POST', '/api/ai-governance', { as: admin, org: other.id, body: { aiEnabled: false } }));
    expect(getAiGovernance(other.id).aiEnabled).toBe(false);
    expect(getAiGovernance('org_default').aiEnabled).toBe(true);
  });
});

describe('features without a backend', () => {
  it('answer 501 instead of storing data in memory', async () => {
    const admin = makeUser('ops@x.org', 'org_admin');
    const cases: [string, Promise<Response>][] = [
      ['webhooks list', webhooks.GET(await req('GET', '/api/webhooks', { as: admin }))],
      ['webhooks create', webhooks.POST(await req('POST', '/api/webhooks', { as: admin, body: { url: 'https://x', events: [] } }))],
      ['webhook delete', webhookById.DELETE(await req('DELETE', '/api/webhooks/w1', { as: admin }), P('w1'))],
      ['sync', sync.GET(await req('GET', '/api/sync', { as: admin }))],
      ['transfers', transfers.GET(await req('GET', '/api/transfers', { as: admin }))],
      ['connectors', connectors.GET(await req('GET', '/api/connectors', { as: admin }))],
    ];
    for (const [name, p] of cases) {
      const res = await p;
      expect(res.status, name).toBe(501);
      expect(((await res.json()) as { error: string }).error).toMatch(/not available yet/);
    }
  });
});
