/**
 * Organization branding: the name, color and logo the console shows to the
 * organization's members, and the look and wording of the email it sends
 * them (invitations, password resets, alerts, scheduled reports).
 *
 * - Child organizations inherit their parent's branding unless they set
 *   their own (a detached organization does not inherit).
 * - Email templates are plain text with `{{placeholders}}`; the defaults are
 *   the console's standard wording. Email channels also get an HTML version
 *   in the brand's colors with the logo attached inline (`cid:`), plus the
 *   footer.
 */
import { db, now, parseJson, type Row } from './db';
import { ancestors, getOrg, OrgError } from './db/repos/orgs';

export const EMAIL_TEMPLATES = {
  invitation: {
    label: 'Invitation',
    vars: ['brand', 'organization', 'inviter', 'link', 'email'],
    subject: 'You are invited to the {{brand}} backup console',
    body: '{{inviter}} invited you to the {{brand}} backup console.\n\nOpen this link within 7 days to accept:\n{{link}}\n\nIf you did not expect this, ignore this email.',
  },
  password_reset: {
    label: 'Password reset',
    vars: ['brand', 'email', 'link'],
    subject: 'Reset your {{brand}} console password',
    body: 'Someone asked to reset the password of {{email}} for the {{brand}} backup console.\n\nOpen this link within an hour to choose a new password:\n{{link}}\n\nIf this was not you, ignore this email; your password stays the same.',
  },
  alert: {
    label: 'Alert',
    vars: ['brand', 'organization', 'severity', 'message', 'machine', 'backup', 'rule', 'time', 'details'],
    subject: '[{{brand}}] {{severity}}: {{message}}',
    body: '{{message}}\n\nMachine: {{machine}}\nBackup: {{backup}}\nRule: {{rule}}\nTime: {{time}}\n{{details}}',
  },
  alert_resolved: {
    label: 'Alert resolved',
    vars: ['brand', 'organization', 'message', 'reason'],
    subject: '[{{brand}}] Resolved: {{message}}',
    body: '{{message}}\n\nResolved: {{reason}}',
  },
  report: {
    label: 'Scheduled report',
    vars: ['brand', 'organization', 'report', 'body'],
    subject: '[{{brand}}] {{report}} — {{organization}}',
    body: '{{body}}',
  },
} as const satisfies Record<string, { label: string; vars: readonly string[]; subject: string; body: string }>;

export type EmailTemplateKey = keyof typeof EMAIL_TEMPLATES;

export interface Branding {
  /** Shown in the console header and used as `{{brand}}` (default "UFOP"). */
  displayName: string;
  /** `#rrggbb`; null = the standard color. */
  primaryColor: string | null;
  /** Sender name for email channels whose From has no name. */
  emailFromName: string;
  /** Plain text appended to every email. */
  emailFooter: string;
  supportEmail: string;
  supportUrl: string;
  templates: Partial<Record<EmailTemplateKey, { subject: string; body: string }>>;
}

export interface EffectiveBranding extends Branding {
  /** Organization whose branding applies (itself or an ancestor); null = defaults. */
  sourceOrgId: string | null;
  inherited: boolean;
  /** `data:image/...;base64,...`, or null. */
  logo: string | null;
}

export const DEFAULT_BRANDING: Branding = { displayName: 'UFOP', primaryColor: null, emailFromName: '', emailFooter: '', supportEmail: '', supportUrl: '', templates: {} };

const LOGO_TYPES = ['image/png', 'image/jpeg', 'image/gif', 'image/webp'];
export const MAX_LOGO_BYTES = 256 * 1024;
const HEX = /^#[0-9a-f]{6}$/i;

function rowOf(orgId: string): Row | undefined {
  return db().prepare('SELECT * FROM org_branding WHERE org_id = ?').get(orgId) as Row | undefined;
}

/** The branding an organization set itself (null = none). */
export function ownBranding(orgId: string): (Branding & { logo: string | null; updatedBy: string; updatedAt: string }) | null {
  const r = rowOf(orgId);
  if (!r) return null;
  return { ...DEFAULT_BRANDING, ...parseJson<Partial<Branding>>(r.settings, {}), logo: (r.logo as string) ?? null, updatedBy: r.updated_by as string, updatedAt: r.updated_at as string };
}

/** The branding in force for an organization: its own, else the nearest ancestor's (not across a detached boundary). */
export function effectiveBranding(orgId: string): EffectiveBranding {
  const chain = [getOrg(orgId), ...ancestors(orgId)].filter(Boolean);
  for (let i = 0; i < chain.length; i++) {
    const o = chain[i]!;
    const b = ownBranding(o.id);
    if (b) {
      const { updatedBy: _u, updatedAt: _a, ...rest } = b;
      return { ...rest, sourceOrgId: o.id, inherited: o.id !== orgId };
    }
    if (o.detached) break;
  }
  return { ...DEFAULT_BRANDING, logo: null, sourceOrgId: null, inherited: false };
}

const text = (v: unknown, what: string, max: number): string => {
  if (v === null || v === undefined) return '';
  if (typeof v !== 'string') throw new OrgError(`${what} must be text`);
  const t = v.trim();
  if (t.length > max) throw new OrgError(`${what} is limited to ${max} characters`);
  return t;
};

/** Placeholders in a template that the template does not offer. */
export function unknownPlaceholders(key: EmailTemplateKey, s: string): string[] {
  const allowed = new Set<string>(EMAIL_TEMPLATES[key].vars);
  return [...new Set([...s.matchAll(/\{\{\s*([a-zA-Z_]+)\s*\}\}/g)].map((m) => m[1]).filter((v) => !allowed.has(v)))];
}

/** Validate a logo data URL (PNG, JPEG, GIF or WebP up to 256 KB). */
export function checkLogo(v: unknown): string | null {
  if (v === null || v === undefined || v === '') return null;
  const m = typeof v === 'string' ? /^data:([a-z/+.-]+);base64,([A-Za-z0-9+/=]+)$/.exec(v) : null;
  if (!m || !LOGO_TYPES.includes(m[1])) throw new OrgError('The logo must be a PNG, JPEG, GIF or WebP image');
  const bytes = Buffer.from(m[2], 'base64');
  if (bytes.length > MAX_LOGO_BYTES) throw new OrgError('The logo is limited to 256 KB');
  // The content must match its declared type (no smuggled SVG/HTML).
  const sig: Record<string, (b: Buffer) => boolean> = {
    'image/png': (b) => b.subarray(0, 8).equals(Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a])),
    'image/jpeg': (b) => b[0] === 0xff && b[1] === 0xd8,
    'image/gif': (b) => b.subarray(0, 4).toString('latin1') === 'GIF8',
    'image/webp': (b) => b.subarray(0, 4).toString('latin1') === 'RIFF' && b.subarray(8, 12).toString('latin1') === 'WEBP',
  };
  if (!sig[m[1]](bytes)) throw new OrgError('The logo file does not match its image type');
  return v as string;
}

/** Validate and store an organization's branding (`null` removes it, so it inherits again). */
export function setBranding(orgId: string, raw: unknown, by: string): EffectiveBranding {
  if (!getOrg(orgId)) throw new OrgError('Organization not found', 404);
  if (raw === null) {
    db().prepare('DELETE FROM org_branding WHERE org_id = ?').run(orgId);
    return effectiveBranding(orgId);
  }
  const b = (raw && typeof raw === 'object' ? raw : {}) as Record<string, unknown>;
  const color = b.primaryColor === null || b.primaryColor === undefined || b.primaryColor === '' ? null : String(b.primaryColor);
  if (color !== null && !HEX.test(color)) throw new OrgError('The color must look like #2563eb');
  const supportEmail = text(b.supportEmail, 'The support email', 200);
  if (supportEmail && !/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(supportEmail)) throw new OrgError('The support email is not an email address');
  const supportUrl = text(b.supportUrl, 'The support link', 500);
  if (supportUrl && !/^https?:\/\/\S+$/i.test(supportUrl)) throw new OrgError('The support link must start with https://');
  const templates: Branding['templates'] = {};
  const rawT = (b.templates && typeof b.templates === 'object' ? b.templates : {}) as Record<string, unknown>;
  for (const [k, v] of Object.entries(rawT)) {
    if (!(k in EMAIL_TEMPLATES)) throw new OrgError(`Unknown email template ${k}`);
    if (!v) continue;
    const key = k as EmailTemplateKey;
    const t = v as Record<string, unknown>;
    const subject = text(t.subject, `${EMAIL_TEMPLATES[key].label} subject`, 200);
    const body = text(t.body, `${EMAIL_TEMPLATES[key].label} text`, 10_000);
    if (!subject || !body) throw new OrgError(`${EMAIL_TEMPLATES[key].label}: give a subject and a text (or reset it to the default)`);
    const unknown = unknownPlaceholders(key, `${subject} ${body}`);
    if (unknown.length) throw new OrgError(`${EMAIL_TEMPLATES[key].label}: unknown placeholder ${unknown.map((u) => `{{${u}}}`).join(', ')} (use ${EMAIL_TEMPLATES[key].vars.map((x) => `{{${x}}}`).join(', ')})`);
    if ((key === 'invitation' || key === 'password_reset') && !/\{\{\s*link\s*\}\}/.test(body)) throw new OrgError(`${EMAIL_TEMPLATES[key].label}: the text must contain {{link}}`);
    templates[key] = { subject, body };
  }
  const settings: Branding = {
    displayName: text(b.displayName, 'The name', 80) || DEFAULT_BRANDING.displayName,
    primaryColor: color,
    emailFromName: text(b.emailFromName, 'The sender name', 80),
    emailFooter: text(b.emailFooter, 'The email footer', 1000),
    supportEmail,
    supportUrl,
    templates,
  };
  const logo = checkLogo(b.logo);
  db()
    .prepare(
      'INSERT INTO org_branding (org_id, settings, logo, updated_by, updated_at) VALUES (?, ?, ?, ?, ?) ON CONFLICT(org_id) DO UPDATE SET settings = excluded.settings, logo = excluded.logo, updated_by = excluded.updated_by, updated_at = excluded.updated_at',
    )
    .run(orgId, JSON.stringify(settings), logo, by, now());
  return effectiveBranding(orgId);
}

/** Replace `{{name}}` placeholders; unknown ones become empty. */
export function fill(template: string, vars: Record<string, string | number | undefined | null>): string {
  return template.replace(/\{\{\s*([a-zA-Z_]+)\s*\}\}/g, (_, k: string) => (vars[k] === undefined || vars[k] === null ? '' : String(vars[k])));
}

/** Subject and text of a templated email for an organization (its template, else the default). */
export function renderTemplate(orgId: string, key: EmailTemplateKey, vars: Record<string, string | number | undefined | null>): { subject: string; text: string } {
  const b = effectiveBranding(orgId);
  const t = b.templates[key] ?? EMAIL_TEMPLATES[key];
  const all = { brand: b.displayName, organization: getOrg(orgId)?.name ?? '', ...vars };
  // Subjects are one line.
  return {
    subject: fill(t.subject, all)
      .replace(/[\r\n]+/g, ' ')
      .trim(),
    text: fill(t.body, all),
  };
}

const esc = (s: string) => s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;').replace(/"/g, '&quot;');

/** Readable text color on `hex`. */
function onColor(hex: string): string {
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 255, (n >> 8) & 255, n & 255];
  return 0.299 * r + 0.587 * g + 0.114 * b > 160 ? '#111111' : '#ffffff';
}

export interface BrandedEmail {
  html: string;
  /** The footer appended to the plain text. */
  text: string;
  /** The logo, attached inline (`cid:brand-logo`). */
  attachments: { filename: string; content: Buffer; contentType: string; cid: string }[];
  fromName: string;
}

/** The HTML version of an email in an organization's branding (links clickable, text escaped). */
export function brandedEmail(orgId: string, subject: string, body: string): BrandedEmail {
  const b = effectiveBranding(orgId);
  const color = b.primaryColor ?? '#2563eb';
  const textWithFooter = b.emailFooter ? `${body}\n\n--\n${b.emailFooter}` : body;
  const linkify = (s: string) => esc(s).replace(/https?:\/\/[^\s<>"]+/g, (u) => `<a href="${u}" style="color:${color}">${u}</a>`);
  const paragraphs = body
    .split(/\n{2,}/)
    .map((p) => `<p style="margin:0 0 14px">${linkify(p).replace(/\n/g, '<br>')}</p>`)
    .join('');
  const logo = b.logo ? /^data:([^;]+);base64,(.+)$/.exec(b.logo) : null;
  const header = logo
    ? `<img src="cid:brand-logo" alt="${esc(b.displayName)}" style="max-height:40px;max-width:200px">`
    : `<span style="font-size:18px;font-weight:600;color:${onColor(color)}">${esc(b.displayName)}</span>`;
  const support = [
    b.supportEmail && `<a href="mailto:${esc(b.supportEmail)}" style="color:#6b7280">${esc(b.supportEmail)}</a>`,
    b.supportUrl && `<a href="${esc(b.supportUrl)}" style="color:#6b7280">${esc(b.supportUrl)}</a>`,
  ]
    .filter(Boolean)
    .join(' · ');
  const html = `<!doctype html><html><head><meta charset="utf-8"><meta name="viewport" content="width=device-width"><title>${esc(subject)}</title></head>
<body style="margin:0;background:#f4f5f7;font-family:-apple-system,Segoe UI,Roboto,Helvetica,Arial,sans-serif;color:#1f2328">
<table role="presentation" width="100%" cellpadding="0" cellspacing="0"><tr><td align="center" style="padding:24px 12px">
<table role="presentation" width="600" cellpadding="0" cellspacing="0" style="max-width:600px;width:100%;background:#ffffff;border-radius:8px;overflow:hidden;border:1px solid #e5e7eb">
<tr><td style="background:${logo ? '#ffffff' : color};padding:16px 24px;border-bottom:3px solid ${color}">${header}</td></tr>
<tr><td style="padding:24px;font-size:15px;line-height:1.5">${paragraphs}</td></tr>
${b.emailFooter || support ? `<tr><td style="padding:16px 24px;background:#f9fafb;font-size:12px;color:#6b7280">${b.emailFooter ? `${linkify(b.emailFooter).replace(/\n/g, '<br>')}` : ''}${b.emailFooter && support ? '<br>' : ''}${support}</td></tr>` : ''}
</table></td></tr></table></body></html>`;
  return {
    html,
    text: textWithFooter,
    attachments: logo ? [{ filename: `logo.${logo[1].split('/')[1]}`, content: Buffer.from(logo[2], 'base64'), contentType: logo[1], cid: 'brand-logo' }] : [],
    fromName: b.emailFromName || (b.sourceOrgId ? b.displayName : ''),
  };
}

/** Example values for previews and test emails. */
export const SAMPLE_VARS: Record<EmailTemplateKey, Record<string, string>> = {
  invitation: { inviter: 'Alex Admin', link: 'https://console.example.com/invite/EXAMPLE', email: 'new.member@example.com' },
  password_reset: { email: 'member@example.com', link: 'https://console.example.com/reset-password?token=EXAMPLE' },
  alert: {
    severity: 'Critical',
    message: 'Backup "Documents" failed on LAPTOP-42',
    machine: 'LAPTOP-42',
    backup: 'Documents',
    rule: 'Failed backups',
    time: '2026-01-01T09:00:00.000Z',
    details: 'error: destination unreachable',
  },
  alert_resolved: { message: 'Backup "Documents" failed on LAPTOP-42', reason: 'the next backup succeeded' },
  report: { report: 'Weekly backup status', body: 'Machines: 42\nSucceeded: 40\nFailed: 2' },
};

/** A template rendered with example values (`draft` replaces the saved subject/text). */
export function previewTemplate(orgId: string, key: EmailTemplateKey, draft?: { subject?: string; body?: string }) {
  const b = effectiveBranding(orgId);
  const base = b.templates[key] ?? EMAIL_TEMPLATES[key];
  const t = { subject: draft?.subject ?? base.subject, body: draft?.body ?? base.body };
  const unknown = unknownPlaceholders(key, `${t.subject} ${t.body}`);
  if (unknown.length) throw new OrgError(`Unknown placeholder ${unknown.map((u) => `{{${u}}}`).join(', ')}`);
  const vars = { brand: b.displayName, organization: getOrg(orgId)?.name ?? '', ...SAMPLE_VARS[key] };
  const subject = fill(t.subject, vars)
    .replace(/[\r\n]+/g, ' ')
    .trim();
  const text = fill(t.body, vars);
  const branded = brandedEmail(orgId, subject, text);
  return { subject, text: branded.text, html: branded.html.replace('cid:brand-logo', b.logo ?? '') };
}
