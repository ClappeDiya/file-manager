/**
 * Password rules for console accounts: at least 12 characters, not built
 * from the account's own name or address, not a common or trivially
 * patterned password, and either three character classes or a long
 * passphrase (16+). The score (0–4) drives the strength meter.
 */

export const MIN_LENGTH = 12;
const PASSPHRASE_LENGTH = 16;

/** Frequently used passwords and fragments (lower-case, compared after stripping digits/symbols). */
const COMMON = new Set([
  'password', 'passw0rd', 'qwerty', 'qwertyuiop', 'letmein', 'welcome', 'admin', 'administrator', 'iloveyou', 'monkey', 'dragon', 'football',
  'baseball', 'sunshine', 'princess', 'master', 'shadow', 'superman', 'trustno', 'starwars', 'whatever', 'computer', 'changeme', 'secret',
  'abc', 'abcdef', 'login', 'access', 'hello', 'freedom', 'michael', 'summer', 'winter', 'spring', 'autumn', 'backup', 'ufop', 'company', 'test',
]);

export interface PasswordCheck {
  ok: boolean;
  score: 0 | 1 | 2 | 3 | 4;
  problems: string[];
}

function sequential(s: string): boolean {
  let run = 1;
  for (let i = 1; i < s.length; i++) {
    const d = s.charCodeAt(i) - s.charCodeAt(i - 1);
    run = d === 1 || d === -1 ? run + 1 : 1;
    if (run >= 5) return true;
  }
  return false;
}

export function checkPassword(pw: string, ctx: { email?: string; name?: string } = {}): PasswordCheck {
  const problems: string[] = [];
  const lower = pw.toLowerCase();
  if (pw.length < MIN_LENGTH) problems.push(`Use at least ${MIN_LENGTH} characters`);
  const classes = [/[a-z]/, /[A-Z]/, /\d/, /[^A-Za-z0-9]/].filter((r) => r.test(pw)).length;
  if (pw.length < PASSPHRASE_LENGTH && classes < 3) problems.push(`Mix upper case, lower case, digits and symbols, or use ${PASSPHRASE_LENGTH}+ characters`);
  if (/(.)\1{3,}/.test(pw)) problems.push('Avoid repeating the same character');
  if (sequential(lower)) problems.push('Avoid sequences like "12345" or "abcde"');
  const core = lower.replace(/[^a-z]/g, '');
  if (COMMON.has(core) || (core.length >= 4 && [...COMMON].some((c) => c.length >= 5 && core === c))) problems.push('This is a commonly used password');
  const local = ctx.email?.split('@')[0]?.toLowerCase().replace(/[^a-z0-9]/g, '') ?? '';
  const names = (ctx.name ?? '').toLowerCase().split(/\s+/).filter((n) => n.length >= 4);
  if ((local.length >= 4 && lower.includes(local)) || names.some((n) => lower.includes(n))) problems.push('Do not use your name or e-mail address');
  const raw = Math.min(4, Math.floor(pw.length / 6) + classes - 2);
  const score = (problems.length ? Math.min(1, Math.max(0, raw)) : Math.max(2, Math.min(4, raw))) as PasswordCheck['score'];
  return { ok: problems.length === 0, score, problems };
}
