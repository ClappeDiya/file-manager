/** Console languages: every locale has every key, placeholders match, the account keeps its language, dates follow it. */
import { afterEach, describe, expect, it } from 'vitest';
import fs from 'fs';
import path from 'path';
import { getUser } from '@/lib/db/repos/users';
import { formatDate, formatRelativeTime, setDisplayLocale, setDisplayTimeZone } from '@/lib/utils/format';
import { json, makeUser, req } from './helpers';
import { isLocale, matchLocale } from '@/lib/i18n';
import * as profile from '@/app/api/account/profile/route';
import * as session from '@/app/api/auth/session/route';

type Tree = { [k: string]: string | Tree };
const dir = path.join(__dirname, '..', 'locales');
const load = (l: string) => JSON.parse(fs.readFileSync(path.join(dir, `${l}.json`), 'utf8')) as Tree;
const flat = (t: Tree, p = ''): Record<string, string> => Object.fromEntries(Object.entries(t).flatMap(([k, v]) => (typeof v === 'string' ? [[`${p}${k}`, v]] : Object.entries(flat(v, `${p}${k}.`)))));
const LOCALES = ['de', 'es', 'fr', 'ja', 'ko', 'pt', 'zh'];

afterEach(() => {
  setDisplayLocale('en');
  setDisplayTimeZone(null);
});

describe('console languages', () => {
  it('has every English key, with the same placeholders, in every locale', () => {
    const en = flat(load('en'));
    expect(
      fs
        .readdirSync(dir)
        .filter((f) => f.endsWith('.json'))
        .sort(),
    ).toEqual(['en', ...LOCALES].map((l) => `${l}.json`).sort());
    for (const l of LOCALES) {
      const t = flat(load(l));
      expect(Object.keys(t).sort(), l).toEqual(Object.keys(en).sort());
      for (const [k, v] of Object.entries(en)) {
        const vars = (s: string) => (s.match(/\{\w+\}/g) ?? []).sort();
        expect(vars(t[k]), `${l} ${k}`).toEqual(vars(v));
        expect(t[k].trim(), `${l} ${k}`).not.toBe('');
      }
    }
  });

  it('picks the best language from the browser', () => {
    expect(matchLocale(['pt-BR', 'en-US'])).toBe('pt');
    expect(matchLocale(['nl-NL', 'de-AT'])).toBe('de');
    expect(matchLocale(['nl'])).toBe('en');
    expect(isLocale('zh')).toBe(true);
    expect(isLocale('xx')).toBe(false);
  });

  it('keeps the account language and returns it with the session', async () => {
    const u = makeUser('lang@x.org', 'operator');
    const set = async (locale: unknown) => json<{ user?: { locale: string | null }; error?: string }>(await profile.PATCH(await req('PATCH', '/api/account/profile', { as: u, body: { locale } })));
    expect((await set('xx')).status).toBe(400);
    expect((await set('ja')).body.user!.locale).toBe('ja');
    const s = await json<{ user: { locale: string } }>(await session.GET(await req('GET', '/api/auth/session', { as: getUser(u.id)! })));
    expect(s.body.user.locale).toBe('ja');
    expect((await set(null)).body.user!.locale).toBeNull();
  });

  it('formats dates and relative times in the console language', () => {
    setDisplayTimeZone('UTC');
    const at = '2026-03-05T10:00:00Z';
    setDisplayLocale('de');
    expect(formatDate(at)).toBe('5. März 2026');
    expect(formatRelativeTime(new Date(Date.now() - 5 * 60_000))).toBe('vor 5 Min.');
    setDisplayLocale('ja');
    expect(formatDate(at)).toBe('2026年3月5日');
    setDisplayLocale('en');
    expect(formatDate(at)).toBe('Mar 5, 2026');
    expect(formatRelativeTime(new Date(Date.now() - 5 * 60_000))).toBe('5m ago');
  });
});
