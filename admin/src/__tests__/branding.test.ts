/**
 * Organization branding: validation, inheritance down the organization tree,
 * templated and branded email (captured by an in-process SMTP server and
 * decoded), previews and test sends.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import net from 'net';
import { createOrg, updateOrg } from '@/lib/db/repos/orgs';
import { createChannel } from '@/lib/db/repos/channels';
import { EMAIL_TEMPLATES, effectiveBranding, renderTemplate } from '@/lib/branding';
import { brandTokens, foregroundOn, shade } from '@/lib/brand-colors';
import { json, makeUser, req } from './helpers';
import * as branding from '@/app/api/branding/route';
import * as preview from '@/app/api/branding/preview/route';
import * as testSend from '@/app/api/branding/test/route';
import * as users from '@/app/api/users/route';

// A 1×1 PNG.
const PNG = 'data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==';

// ── Minimal SMTP capture (no TLS, no auth) ──
let smtp: net.Server;
let smtpPort = 0;
const mails: { from: string; rcpt: string[]; data: string }[] = [];

beforeAll(async () => {
  smtp = net.createServer((sock) => {
    let buf = '';
    let inData = false;
    let cur = { from: '', rcpt: [] as string[], data: '' };
    sock.write('220 test ESMTP\r\n');
    sock.on('data', (chunk) => {
      buf += chunk.toString('latin1');
      for (;;) {
        if (inData) {
          const end = buf.indexOf('\r\n.\r\n');
          if (end < 0) return;
          cur.data = buf.slice(0, end);
          buf = buf.slice(end + 5);
          inData = false;
          mails.push(cur);
          cur = { from: '', rcpt: [], data: '' };
          sock.write('250 queued\r\n');
          continue;
        }
        const nl = buf.indexOf('\r\n');
        if (nl < 0) return;
        const line = buf.slice(0, nl);
        buf = buf.slice(nl + 2);
        const cmd = line.slice(0, 4).toUpperCase();
        if (cmd === 'EHLO' || cmd === 'HELO') sock.write('250-test\r\n250 8BITMIME\r\n');
        else if (cmd === 'MAIL') {
          cur.from = line;
          sock.write('250 ok\r\n');
        } else if (cmd === 'RCPT') {
          cur.rcpt.push(line);
          sock.write('250 ok\r\n');
        } else if (cmd === 'DATA') {
          inData = true;
          sock.write('354 go\r\n');
        }
        else if (cmd === 'QUIT') sock.end('221 bye\r\n');
        else sock.write('250 ok\r\n');
      }
    });
  });
  await new Promise<void>((r) => smtp.listen(0, '127.0.0.1', r));
  smtpPort = (smtp.address() as net.AddressInfo).port;
});
afterAll(() => smtp.close());

/** Decode quoted-printable soft breaks and escapes. */
const qp = (s: string) => s.replace(/=\r\n/g, '').replace(/=([0-9A-F]{2})/g, (_, h) => String.fromCharCode(parseInt(h, 16)));
/** A header value, unfolded, with RFC 2047 encoded words decoded. */
const header = (raw: string, name: string) =>
  (new RegExp(`^${name}: (.*(?:\\r\\n[ \\t].*)*)`, 'mi').exec(raw)?.[1].replace(/\r\n[ \t]/g, ' ') ?? '')
    .replace(/\?=\s+=\?/g, '?==?')
    .replace(/=\?UTF-8\?Q\?(.*?)\?=/gi, (_, w: string) => Buffer.from(qp(w.replace(/_/g, ' ')), 'latin1').toString('utf8'));

function emailChannel(orgId: string, from = 'backups@acme.test') {
  return createChannel(orgId, { name: 'Mail', kind: 'email', config: { host: '127.0.0.1', port: smtpPort, security: 'none', username: '', from, to: ['ops@acme.test'], useForInvitations: true } });
}

describe('branding settings', () => {
  it('validates, inherits down the tree, stops at detached organizations and resets', async () => {
    const parent = createOrg({ name: 'Acme' });
    const child = createOrg({ name: 'Acme EU', parentId: parent.id });
    const detached = createOrg({ name: 'Acme Labs', parentId: parent.id });
    updateOrg(detached.id, { detached: true });
    const admin = makeUser('admin@acme.test', 'org_admin', parent.id);
    const member = makeUser('member@acme.test', 'user', child.id);
    const put = async (body: unknown) => json<{ branding?: { displayName: string }; error?: string }>(await branding.PUT(await req('PUT', '/api/branding', { as: admin, body })));

    expect((await json<{ canEdit: boolean; branding: { displayName: string } }>(await branding.GET(await req('GET', '/api/branding', { as: member })))).body).toMatchObject({
      canEdit: false,
      branding: { displayName: 'UFOP' },
    });
    expect((await branding.PUT(await req('PUT', '/api/branding', { as: member, body: {} }))).status).toBe(403);
    for (const [bad, msg] of [
      [{ primaryColor: 'blue' }, '#2563eb'],
      [{ logo: 'data:image/svg+xml;base64,PHN2Zz48L3N2Zz4=' }, 'PNG, JPEG'],
      [{ logo: 'data:image/png;base64,R0lGODlhAQABAAAAACw=' }, 'does not match'],
      [{ supportUrl: 'javascript:alert(1)' }, 'https://'],
      [{ templates: { invitation: { subject: 'Hi', body: 'No link here' } } }, '{{link}}'],
      [{ templates: { alert: { subject: '{{severity}} {{secret}}', body: 'x' } } }, '{{secret}}'],
      [{ templates: { nope: { subject: 'a', body: 'b' } } }, 'Unknown email template'],
    ] as const) {
      const r = await put(bad);
      expect(r.status, JSON.stringify(bad)).toBe(400);
      expect(r.body.error).toContain(msg);
    }
    const ok = await put({
      displayName: 'Acme Backup',
      primaryColor: '#0a7d55',
      logo: PNG,
      emailFromName: 'Acme IT',
      emailFooter: 'Acme Corp · IT Department',
      supportEmail: 'help@acme.test',
      supportUrl: 'https://help.acme.test',
      templates: { invitation: { subject: 'Join {{brand}} ({{organization}})', body: 'Hello! {{inviter}} added you.\n\nAccept here: {{link}}' } },
    });
    expect(ok.status).toBe(200);
    expect(effectiveBranding(child.id)).toMatchObject({ displayName: 'Acme Backup', inherited: true, sourceOrgId: parent.id, primaryColor: '#0a7d55' });
    expect(effectiveBranding(detached.id)).toMatchObject({ displayName: 'UFOP', sourceOrgId: null });
    const seen = await json<{ branding: { displayName: string; logo: string; inherited: boolean } }>(await branding.GET(await req('GET', '/api/branding', { as: member })));
    expect(seen.body.branding).toMatchObject({ displayName: 'Acme Backup', logo: PNG, inherited: true });

    expect((await branding.DELETE(await req('DELETE', '/api/branding', { as: admin }))).status).toBe(200);
    expect(effectiveBranding(child.id).displayName).toBe('UFOP');
  });

  it('keeps the standard wording by default (templates reproduce it)', () => {
    const org = createOrg({ name: 'Plain' });
    expect(renderTemplate(org.id, 'invitation', { inviter: 'Ann', link: 'https://x/i/1' })).toEqual({
      subject: 'You are invited to the UFOP backup console',
      text: 'Ann invited you to the UFOP backup console.\n\nOpen this link within 7 days to accept:\nhttps://x/i/1\n\nIf you did not expect this, ignore this email.',
    });
    expect(renderTemplate(org.id, 'alert', { severity: 'Critical', message: 'm', machine: 'M', backup: 'b', rule: 'r', time: 't', details: 'd: 1' })).toEqual({
      subject: '[UFOP] Critical: m',
      text: 'm\n\nMachine: M\nBackup: b\nRule: r\nTime: t\nd: 1',
    });
    expect(Object.keys(EMAIL_TEMPLATES)).toEqual(['invitation', 'password_reset', 'alert', 'alert_resolved', 'report']);
  });
});

describe('branded email', () => {
  it('sends invitations with the template, sender name, HTML in the brand color, inline logo and footer', async () => {
    const org = createOrg({ name: 'Globex' });
    const admin = makeUser('admin@globex.test', 'org_admin', org.id);
    emailChannel(org.id);
    await branding.PUT(
      await req('PUT', '/api/branding', {
        as: admin,
        body: {
          displayName: 'Globex Vault',
          primaryColor: '#0a7d55',
          logo: PNG,
          emailFromName: 'Globex IT',
          emailFooter: 'Globex Corp — internal use',
          templates: { invitation: { subject: 'Join {{brand}} at {{organization}}', body: '{{inviter}} added you to {{brand}}.\n\nAccept: {{link}}' } },
        },
      }),
    );
    const before = mails.length;
    const r = await json<{ emailed: boolean; link: string }>(await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'newbie@globex.test', role: 'operator' } })));
    expect(r.body.emailed).toBe(true);
    const m = mails[before];
    expect(m.rcpt.join()).toContain('newbie@globex.test');
    const raw = m.data;
    expect(header(raw, 'Subject')).toBe('Join Globex Vault at Globex');
    expect(header(raw, 'From')).toMatch(/^"?Globex IT"? <backups@acme\.test>$/);
    expect(raw).toMatch(/multipart\/alternative/);
    const decoded = qp(raw);
    expect(decoded).toContain(`added you to Globex Vault.`);
    expect(decoded).toContain(r.body.link);
    expect(decoded).toContain('Globex Corp');
    expect(decoded).toContain('internal use');
    expect(decoded).toContain('#0a7d55');
    expect(decoded).toContain('src="cid:brand-logo"');
    expect(decoded).toContain(`<a href="${r.body.link}"`);
    expect(raw).toMatch(/Content-ID: <brand-logo>/i);
  });

  it('leaves unbranded organizations plain text with the standard wording', async () => {
    const org = createOrg({ name: 'Initech' });
    const admin = makeUser('admin@initech.test', 'org_admin', org.id);
    emailChannel(org.id, '"Initech Ops" <ops@initech.test>');
    const before = mails.length;
    await users.POST(await req('POST', '/api/users', { as: admin, body: { email: 'peter@initech.test', role: 'operator' } }));
    const raw = mails[before].data;
    expect(header(raw, 'Subject')).toBe('You are invited to the UFOP backup console');
    expect(raw).not.toMatch(/multipart\/alternative|text\/html/);
    expect(header(raw, 'From')).toMatch(/^"?Initech Ops"? <ops@initech\.test>$/);
  });

  it('previews templates with example values and sends a test to the editor', async () => {
    const org = createOrg({ name: 'Hooli' });
    const admin = makeUser('admin@hooli.test', 'org_admin', org.id);
    const ch = emailChannel(org.id);
    await branding.PUT(
      await req('PUT', '/api/branding', { as: admin, body: { displayName: 'Hooli Box', logo: PNG, templates: { alert: { subject: '{{brand}} alert: {{message}}', body: 'Machine {{machine}} — {{details}}' } } } }),
    );
    const p = await json<{ subject: string; text: string; html: string }>(await preview.POST(await req('POST', '/api/branding/preview', { as: admin, body: { template: 'alert' } })));
    expect(p.body.subject).toBe('Hooli Box alert: Backup "Documents" failed on LAPTOP-42');
    expect(p.body.text).toContain('Machine LAPTOP-42');
    expect(p.body.html).toContain(`src="${PNG}"`);
    expect(p.body.html).toContain('&quot;Documents&quot;');
    const draft = await json<{ subject: string }>(await preview.POST(await req('POST', '/api/branding/preview', { as: admin, body: { template: 'alert', subject: 'Draft {{severity}}' } })));
    expect(draft.body.subject).toBe('Draft Critical');
    expect((await preview.POST(await req('POST', '/api/branding/preview', { as: admin, body: { template: 'alert', subject: '{{nope}}' } }))).status).toBe(400);

    const before = mails.length;
    const t = await json<{ ok: boolean }>(await testSend.POST(await req('POST', '/api/branding/test', { as: admin, body: { template: 'alert', channelId: ch.id } })));
    expect(t.body.ok).toBe(true);
    expect(mails[before].rcpt.join()).toContain('admin@hooli.test');
    expect(header(mails[before].data, 'Subject')).toBe('Hooli Box alert: Backup "Documents" failed on LAPTOP-42');
    expect((await testSend.POST(await req('POST', '/api/branding/test', { as: admin, body: { template: 'alert', channelId: 'nope' } }))).status).toBe(400);
  });

  it('derives readable brand tokens', () => {
    expect(brandTokens('#0a7d55')).toEqual({ '--color-primary': '#0a7d55', '--color-primary-hover': shade('#0a7d55', -0.12), '--color-primary-active': shade('#0a7d55', -0.24), '--color-primary-foreground': '#ffffff' });
    expect(foregroundOn('#fde047')).toBe('#111111');
    expect(shade('#ffffff', -0.5)).toBe('#808080');
    expect(brandTokens('red')).toBeNull();
  });
});
