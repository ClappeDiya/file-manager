/**
 * SSO end to end against real implementations: a live `oidc-provider`
 * authorization server (discovery, PKCE, signed ID tokens, userinfo) and
 * SAML responses signed by a samlify identity provider.
 */
import { afterAll, beforeAll, describe, expect, it } from 'vitest';
import http from 'http';
import type { AddressInfo } from 'net';
import crypto from 'crypto';
import fs from 'fs';
import os from 'os';
import path from 'path';
import zlib from 'zlib';
import { execFileSync } from 'child_process';
import { NextRequest } from 'next/server';
import Provider from 'oidc-provider';
import * as samlify from 'samlify';
import { createProvider, identitiesOf } from '@/lib/db/repos/sso';
import { createGroup, groupMembers } from '@/lib/db/repos/groups';
import { findUserByEmail, getUser } from '@/lib/db/repos/users';
import { effectiveRole } from '@/lib/db/repos/orgs';
import { hotp, totpStep } from '@/lib/totp';
import { cookieFrom, json, makeUser, req } from './helpers';
import * as start from '@/app/api/auth/sso/[id]/start/route';
import * as callback from '@/app/api/auth/sso/[id]/callback/route';
import * as metadata from '@/app/api/auth/sso/[id]/metadata/route';
import * as discoverRoute from '@/app/api/auth/sso/discover/route';
import * as ssoAdmin from '@/app/api/sso/route';
import * as identities from '@/app/api/account/identities/route';
import * as accountMfa from '@/app/api/account/mfa/route';
import * as mfaVerify from '@/app/api/auth/mfa/verify/route';

const ORIGIN = 'http://localhost:3001';
const params = (id: string) => ({ params: Promise.resolve({ id }) });

// ── OIDC: a real authorization server on a random port ──

const accounts: Record<string, Record<string, unknown>> = {
  alice: { sub: 'alice', email: 'alice@corp.example', email_verified: true, name: 'Alice Doe', groups: ['eng', 'unmapped'] },
  bob: { sub: 'bob', email: 'bob@corp.example', email_verified: false, name: 'Bob' },
  carol: { sub: 'carol', email: 'carol@corp.example', email_verified: true, name: 'Carol' },
};
let issuer = '';
let nextLogin = '';
let server: http.Server;

beforeAll(async () => {
  server = http.createServer();
  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  issuer = `http://127.0.0.1:${(server.address() as AddressInfo).port}`;
  const oidc = new Provider(issuer, {
    clients: [{ client_id: 'ufop-console', client_secret: 'console-secret', redirect_uris: [`${ORIGIN}/api/auth/sso/placeholder/callback`], grant_types: ['authorization_code'], response_types: ['code'] }],
    findAccount: async (_ctx, sub) => (accounts[sub] ? { accountId: sub, claims: async () => accounts[sub] as { sub: string } } : undefined),
    claims: { openid: ['sub'], email: ['email', 'email_verified'], profile: ['name'], groups: ['groups'] },
    scopes: ['openid', 'email', 'profile', 'groups', 'offline_access'],
    features: { devInteractions: { enabled: false } },
    pkce: { required: () => true },
    cookies: { keys: ['test-cookie-key'] },
  });
  // The provider id (and so the redirect URI) is known per test; accept the console's callback path.
  oidc.Client.prototype.redirectUriAllowed = function (uri: string) {
    return uri.startsWith(`${ORIGIN}/api/auth/sso/`) && uri.endsWith('/callback');
  };
  // Interaction: sign in as `nextLogin` and grant the requested scopes.
  const handler = oidc.callback();
  server.on('request', async (rq, rs) => {
    if (!rq.url?.startsWith('/interaction/')) return handler(rq, rs);
    const details = await oidc.interactionDetails(rq, rs);
    const grant = new oidc.Grant({ accountId: nextLogin, clientId: String(details.params.client_id) });
    grant.addOIDCScope(String(details.params.scope));
    await oidc.interactionFinished(rq, rs, { login: { accountId: nextLogin }, consent: { grantId: await grant.save() } }, { mergeWithLastSubmission: false });
  });
});

afterAll(() => new Promise<void>((r) => server.close(() => r())));

/** Minimal cookie-jar browser that stops at the console's callback. */
async function browseToCallback(authorizeUrl: string, login: string): Promise<string> {
  const jar = new Map<string, string>();
  const store = (res: Response) => {
    for (const c of res.headers.getSetCookie()) {
      const [kv] = c.split(';');
      const i = kv.indexOf('=');
      jar.set(kv.slice(0, i), kv.slice(i + 1));
    }
  };
  const cookie = () => [...jar].map(([k, v]) => `${k}=${v}`).join('; ');
  nextLogin = login;
  let url = authorizeUrl;
  let form: Record<string, string> | null = null;
  for (let hop = 0; hop < 20; hop++) {
    if (url.startsWith(ORIGIN)) return url;
    const res = await fetch(url, {
      method: form ? 'POST' : 'GET',
      redirect: 'manual',
      headers: { cookie: cookie(), ...(form ? { 'content-type': 'application/x-www-form-urlencoded' } : {}) },
      body: form ? new URLSearchParams(form).toString() : undefined,
    });
    store(res);
    form = null;
    if (res.status >= 300 && res.status < 400) {
      url = new URL(res.headers.get('location')!, url).toString();
      continue;
    }
    const html = await res.text();
    const action = /<form[^>]*action="([^"]+)"/.exec(html)?.[1];
    if (!action) throw new Error(`Unexpected page at ${url}: ${res.status} ${html.replace(/<style[\s\S]*?<\/style>/, "").replace(/<[^>]+>/g, " ").replace(/\s+/g, " ").slice(0, 400)}`);
    url = new URL(action.replace(/&amp;/g, '&'), url).toString();
    const fields: Record<string, string> = {};
    for (const m of html.matchAll(/<input[^>]*name="([^"]+)"[^>]*?(?:value="([^"]*)")?[^>]*>/g)) fields[m[1]] = m[2] ?? '';
    if ('login' in fields) Object.assign(fields, { login, password: 'any' });
    form = fields;
  }
  throw new Error('Too many redirects');
}

function oidcProvider(over: Record<string, unknown> = {}) {
  return createProvider('org_default', {
    name: 'Corp SSO',
    kind: 'oidc',
    config: { issuer, clientId: 'ufop-console', scopes: 'openid email profile groups' },
    secret: 'console-secret',
    domains: ['corp.example'],
    ...over,
  });
}

async function signInWith(providerId: string, login: string, o: { redirect?: string; cookie?: string } = {}) {
  const q = new URLSearchParams({ redirect: o.redirect ?? '/fleet', ...(o.cookie ? { link: '1' } : {}) });
  const s = await start.GET(await req('GET', `/api/auth/sso/${providerId}/start?${q}`, o.cookie ? { headers: { cookie: o.cookie } } : {}), params(providerId));
  expect(s.status).toBe(303);
  const cb = await browseToCallback(s.headers.get('location')!, login);
  return callback.GET(await req('GET', cb.slice(ORIGIN.length)), params(providerId));
}

describe('OIDC sign-in', () => {
  it('creates the account on first login with the default role, default group and mapped groups', async () => {
    const eng = createGroup('org_default', { name: 'Engineering' });
    const all = createGroup('org_default', { name: 'Everyone' });
    const p = oidcProvider({ defaultRole: 'user', defaultGroupId: all.id, groupMapping: { eng: eng.id } });
    const res = await signInWith(p.id, 'alice');
    expect(res.status).toBe(303);
    expect(res.headers.get('location')).toBe(`${ORIGIN}/fleet`);
    expect(cookieFrom(res)).toBeTruthy();
    const u = findUserByEmail('alice@corp.example')!;
    expect(u.name).toBe('Alice Doe');
    expect(effectiveRole(u.id, 'org_default')).toBe('user');
    expect(groupMembers(eng.id).map((m) => m.userId)).toContain(u.id);
    expect(groupMembers(all.id).map((m) => m.userId)).toContain(u.id);
    expect(identitiesOf(u.id)).toHaveLength(1);
    // Second sign-in reuses the linked identity.
    const again = await signInWith(p.id, 'alice');
    expect(again.headers.get('location')).toBe(`${ORIGIN}/fleet`);
    expect(identitiesOf(u.id)).toHaveLength(1);
  });

  it('refuses unverified email, foreign domains and sign-up when JIT is off', async () => {
    const p = oidcProvider();
    const bob = await signInWith(p.id, 'bob');
    expect(decodeURIComponent(bob.headers.get('location')!)).toContain('did not verify this email');
    expect(findUserByEmail('bob@corp.example')).toBeNull();

    const other = oidcProvider({ name: 'Other', domains: ['other.example'] });
    const r = await signInWith(other.id, 'carol');
    expect(decodeURIComponent(r.headers.get('location')!)).toContain('cannot sign up');

    const noJit = oidcProvider({ name: 'No JIT', allowJit: false });
    const r2 = await signInWith(noJit.id, 'carol');
    expect(decodeURIComponent(r2.headers.get('location')!)).toContain('invite you first');
    expect(cookieFrom(r2)).toBeNull();
  });

  it('links an existing account by verified email in an owned domain only', async () => {
    const existing = makeUser('carol@corp.example', 'operator');
    const anyDomain = oidcProvider({ name: 'Any', domains: [] });
    const refused = await signInWith(anyDomain.id, 'carol');
    expect(decodeURIComponent(refused.headers.get('location')!)).toContain('link this method from your profile');
    const p = oidcProvider();
    const ok = await signInWith(p.id, 'carol');
    expect(ok.headers.get('location')).toBe(`${ORIGIN}/fleet`);
    expect(identitiesOf(existing.id).map((i) => i.providerId)).toEqual([p.id]);
    expect(effectiveRole(existing.id, 'org_default')).toBe('operator');
  });

  it('links from a signed-in session and unlinks only while another method remains', async () => {
    const dana = makeUser('dana@elsewhere.example', 'operator', 'org_default', { passwordHash: '$2a$12$abcdefghijklmnopqrstuuJ0rO4B6pQf1n0B4yqA7xR2vG2J8bY2' });
    const p = oidcProvider();
    const login = await req('GET', '/', { as: dana });
    const res = await signInWith(p.id, 'carol', { cookie: login.headers.get('cookie')! });
    expect(res.headers.get('location')).toBe(`${ORIGIN}/account?linked=Corp%20SSO`);
    const list = await json<{ identities: { id: string; email: string }[]; hasPassword: boolean }>(await identities.GET(await req('GET', '/api/account/identities', { as: dana })));
    expect(list.body.identities.map((i) => i.email)).toEqual(['carol@corp.example']);
    // The IdP subject is now Dana's; signing in with it lands in Dana's account.
    const signIn = await signInWith(p.id, 'carol');
    expect(signIn.status).toBe(303);
    expect(findUserByEmail('carol@corp.example')).toBeNull();

    const del = await identities.DELETE(await req('DELETE', `/api/account/identities?id=${list.body.identities[0].id}`, { as: dana }));
    expect(del.status).toBe(200);

    // Without a password, the last identity cannot be removed.
    const erin = oidcProvider({ name: 'Erin SSO' });
    await signInWith(erin.id, 'alice');
    const alice = findUserByEmail('alice@corp.example')!;
    const only = identitiesOf(alice.id)[0];
    const blocked = await identities.DELETE(await req('DELETE', `/api/account/identities?id=${only.id}`, { as: alice }));
    expect(blocked.status).toBe(409);
  });

  it('rejects a replayed or forged state', async () => {
    const p = oidcProvider();
    const s = await start.GET(await req('GET', `/api/auth/sso/${p.id}/start`), params(p.id));
    const cb = await browseToCallback(s.headers.get('location')!, 'alice');
    const first = await callback.GET(await req('GET', cb.slice(ORIGIN.length)), params(p.id));
    expect(cookieFrom(first)).toBeTruthy();
    const replay = await callback.GET(await req('GET', cb.slice(ORIGIN.length)), params(p.id));
    expect(decodeURIComponent(replay.headers.get('location')!)).toContain('expired');
    const forged = new URL(cb);
    forged.searchParams.set('state', 'not-a-state');
    const f = await callback.GET(await req('GET', forged.pathname + forged.search), params(p.id));
    expect(cookieFrom(f)).toBeNull();
  });

  it('asks for the second factor when the provider requires it', async () => {
    const p = oidcProvider({ requireMfa: true });
    // No TOTP set up: refused.
    const r = await signInWith(p.id, 'alice');
    expect(decodeURIComponent(r.headers.get('location')!)).toContain('requires two-factor');
    const alice = findUserByEmail('alice@corp.example')!;
    // Enroll through the account API, then sign in again.
    const begin = await json<{ secret: string; otpauthUrl: string }>(await accountMfa.POST(await req('POST', '/api/account/mfa', { as: alice, body: { action: 'begin' } })));
    expect(begin.body.otpauthUrl).toMatch(/^otpauth:\/\/totp\//);
    const code = hotp(begin.body.secret, totpStep() - 1);
    const confirm = await json<{ recoveryCodes: string[] }>(await accountMfa.POST(await req('POST', '/api/account/mfa', { as: alice, body: { action: 'confirm', code } })));
    expect(confirm.body.recoveryCodes).toHaveLength(10);
    const r2 = await signInWith(p.id, 'alice');
    const loc = new URL(r2.headers.get('location')!);
    expect(loc.pathname).toBe('/login');
    expect(cookieFrom(r2)).toBeNull();
    const verify = await mfaVerify.POST(await req('POST', '/api/auth/mfa/verify', { body: { challengeId: loc.searchParams.get('mfa'), code: hotp(begin.body.secret, totpStep()) } }));
    expect(verify.status).toBe(200);
    expect(cookieFrom(verify)).toBeTruthy();
    // Disabling requires a fresh, unused code.
    const reused = await accountMfa.POST(await req('POST', '/api/account/mfa', { as: getUser(alice.id)!, body: { action: 'disable', code: hotp(begin.body.secret, totpStep()) } }));
    expect(reused.status).toBe(401);
  });

  it('discovers providers by email domain and keeps secrets out of the admin API', async () => {
    const p = oidcProvider();
    const d = await json<{ providers: { id: string }[] }>(await discoverRoute.GET(await req('GET', '/api/auth/sso/discover?email=Someone@CORP.example')));
    expect(d.body.providers.map((x) => x.id)).toEqual([p.id]);
    const none = await json<{ providers: unknown[] }>(await discoverRoute.GET(await req('GET', '/api/auth/sso/discover?email=x@nowhere.example')));
    expect(none.body.providers).toEqual([]);
    const owner = makeUser('owner@corp.example', 'org_owner');
    const list = await ssoAdmin.GET(await req('GET', '/api/sso', { as: owner }));
    const text = await list.text();
    expect(text).not.toContain('console-secret');
    expect(text).toContain(`/api/auth/sso/${p.id}/callback`);
    const viewer = makeUser('v@corp.example', 'user');
    expect((await ssoAdmin.GET(await req('GET', '/api/sso', { as: viewer }))).status).toBe(403);
    const owners = await ssoAdmin.POST(await req('POST', '/api/sso', { as: owner, body: { name: 'x', kind: 'oidc', config: { issuer, clientId: 'c' }, secret: 's', defaultRole: 'org_owner' } }));
    expect(owners.status).toBe(400);
  });
});

// ── SAML: responses signed by a samlify identity provider ──

describe('SAML sign-in', () => {
  let idpKey = '';
  let idpCert = '';
  const otherKeyPair: { key: string; cert: string } = { key: '', cert: '' };

  function selfSigned(cn: string) {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'ufop-saml-'));
    execFileSync('openssl', ['req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '2', '-subj', `/CN=${cn}`, '-keyout', path.join(dir, 'k.pem'), '-out', path.join(dir, 'c.pem')], { stdio: 'ignore' });
    const out = { key: fs.readFileSync(path.join(dir, 'k.pem'), 'utf8'), cert: fs.readFileSync(path.join(dir, 'c.pem'), 'utf8') };
    fs.rmSync(dir, { recursive: true, force: true });
    return out;
  }

  beforeAll(() => {
    samlify.setSchemaValidator({ validate: async () => 'skipped' });
    ({ key: idpKey, cert: idpCert } = selfSigned('idp.corp.example'));
    Object.assign(otherKeyPair, selfSigned('attacker.example'));
  });

  function idp(keyPair = { key: idpKey, cert: idpCert }) {
    return samlify.IdentityProvider({
      entityID: 'https://idp.corp.example/metadata',
      privateKey: keyPair.key,
      signingCert: keyPair.cert,
      isAssertionEncrypted: false,
      singleSignOnService: [{ Binding: 'urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect', Location: 'https://idp.corp.example/sso' }],
      loginResponseTemplate: {
        context:
          '<samlp:Response xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion" ID="{ID}" Version="2.0" IssueInstant="{IssueInstant}" Destination="{Destination}" InResponseTo="{InResponseTo}"><saml:Issuer>{Issuer}</saml:Issuer><samlp:Status><samlp:StatusCode Value="{StatusCode}"/></samlp:Status><saml:Assertion ID="{AssertionID}" Version="2.0" IssueInstant="{IssueInstant}"><saml:Issuer>{Issuer}</saml:Issuer><saml:Subject><saml:NameID Format="{NameIDFormat}">{NameID}</saml:NameID><saml:SubjectConfirmation Method="urn:oasis:names:tc:SAML:2.0:cm:bearer"><saml:SubjectConfirmationData NotOnOrAfter="{SubjectConfirmationDataNotOnOrAfter}" Recipient="{SubjectRecipient}" InResponseTo="{InResponseTo}"/></saml:SubjectConfirmation></saml:Subject><saml:Conditions NotBefore="{ConditionsNotBefore}" NotOnOrAfter="{ConditionsNotOnOrAfter}"><saml:AudienceRestriction><saml:Audience>{Audience}</saml:Audience></saml:AudienceRestriction></saml:Conditions><saml:AuthnStatement AuthnInstant="{IssueInstant}" SessionNotOnOrAfter="{ConditionsNotOnOrAfter}"><saml:AuthnContext><saml:AuthnContextClassRef>urn:oasis:names:tc:SAML:2.0:ac:classes:PasswordProtectedTransport</saml:AuthnContextClassRef></saml:AuthnContext></saml:AuthnStatement>{AttributeStatement}</saml:Assertion></samlp:Response>',
        attributes: [
          { name: 'mail', valueTag: 'mail', nameFormat: 'urn:oasis:names:tc:SAML:2.0:attrname-format:basic', valueXsiType: 'xs:string' },
          { name: 'displayName', valueTag: 'displayName', nameFormat: 'urn:oasis:names:tc:SAML:2.0:attrname-format:basic', valueXsiType: 'xs:string' },
          { name: 'groups', valueTag: 'groups', nameFormat: 'urn:oasis:names:tc:SAML:2.0:attrname-format:basic', valueXsiType: 'xs:string' },
        ],
      },
    });
  }

  function samlProvider(over: Record<string, unknown> = {}) {
    return createProvider('org_default', {
      name: 'Corp SAML',
      kind: 'saml',
      config: { entryPoint: 'https://idp.corp.example/sso', idpCert, idpIssuer: 'https://idp.corp.example/metadata', emailAttribute: 'mail' },
      domains: ['corp.example'],
      ...over,
    });
  }

  /** Start the flow, then have the IdP answer the AuthnRequest. */
  async function samlResponse(providerId: string, user: { email: string; name: string; groups: string }, o: { keyPair?: { key: string; cert: string }; inResponseTo?: string; mutate?: (xml: string) => string } = {}) {
    const s = await start.GET(await req('GET', `/api/auth/sso/${providerId}/start?redirect=/alerts`), params(providerId));
    const loc = new URL(s.headers.get('location')!);
    expect(loc.origin + loc.pathname).toBe('https://idp.corp.example/sso');
    const authn = zlib.inflateRawSync(Buffer.from(loc.searchParams.get('SAMLRequest')!, 'base64')).toString();
    const requestId = /ID="([^"]+)"/.exec(authn)![1];
    const md = await (await metadata.GET(await req('GET', `/api/auth/sso/${providerId}/metadata`), params(providerId))).text();
    const sp = samlify.ServiceProvider({ metadata: md });
    const tpl = idp(o.keyPair);
    const now = new Date();
    const later = new Date(now.getTime() + 5 * 60_000);
    const { context } = (await tpl.createLoginResponse(sp, { extract: { request: { id: o.inResponseTo ?? requestId } } }, 'post', { email: user.email }, (template: string) => {
      const id = `_${crypto.randomUUID()}`;
      const values: Record<string, string> = {
        ID: id,
        AssertionID: `_${crypto.randomUUID()}`,
        Destination: `${ORIGIN}/api/auth/sso/${providerId}/callback`,
        SubjectRecipient: `${ORIGIN}/api/auth/sso/${providerId}/callback`,
        Audience: `${ORIGIN}/api/auth/sso/${providerId}/metadata`,
        Issuer: 'https://idp.corp.example/metadata',
        IssueInstant: now.toISOString(),
        StatusCode: 'urn:oasis:names:tc:SAML:2.0:status:Success',
        NameIDFormat: 'urn:oasis:names:tc:SAML:1.1:nameid-format:emailAddress',
        NameID: user.email,
        InResponseTo: o.inResponseTo ?? requestId,
        ConditionsNotBefore: new Date(now.getTime() - 60_000).toISOString(),
        ConditionsNotOnOrAfter: later.toISOString(),
        SubjectConfirmationDataNotOnOrAfter: later.toISOString(),
        attrMail: user.email,
        attrDisplayName: user.name,
        attrGroups: user.groups,
      };
      return { id, context: template.replace(/\{(\w+)\}/g, (m, k: string) => values[k] ?? m) };
    })) as { context: string };
    const xml = Buffer.from(context, 'base64').toString();
    const posted = o.mutate ? Buffer.from(o.mutate(xml)).toString('base64') : context;
    const form = new URLSearchParams({ SAMLResponse: posted, RelayState: loc.searchParams.get('RelayState')! });
    const post = new NextRequest(new URL(`/api/auth/sso/${providerId}/callback`, ORIGIN), { method: 'POST', headers: { 'content-type': 'application/x-www-form-urlencoded', origin: 'https://idp.corp.example' }, body: form.toString() });
    return callback.POST(post, params(providerId));
  }

  it('publishes SP metadata that asks for signed assertions', async () => {
    const p = samlProvider();
    const md = await (await metadata.GET(await req('GET', `/api/auth/sso/${p.id}/metadata`), params(p.id))).text();
    expect(md).toContain('WantAssertionsSigned="true"');
    expect(md).toContain(`${ORIGIN}/api/auth/sso/${p.id}/callback`);
  });

  it('signs in with a signed assertion and assigns mapped groups', async () => {
    const ops = createGroup('org_default', { name: 'Ops' });
    const p = samlProvider({ defaultRole: 'operator', groupMapping: { 'ops-team': ops.id } });
    const res = await samlResponse(p.id, { email: 'sam@corp.example', name: 'Sam Saml', groups: 'ops-team' });
    expect(res.headers.get('location')).toBe(`${ORIGIN}/alerts`);
    expect(cookieFrom(res)).toBeTruthy();
    const u = findUserByEmail('sam@corp.example')!;
    expect(u.ssoProvider).toBe('saml');
    expect(effectiveRole(u.id, 'org_default')).toBe('operator');
    expect(groupMembers(ops.id).map((m) => m.userId)).toEqual([u.id]);
  });

  it('rejects a response signed by another key, a tampered assertion and an unsolicited response', async () => {
    const p = samlProvider();
    const forged = await samlResponse(p.id, { email: 'eve@corp.example', name: 'Eve', groups: '' }, { keyPair: otherKeyPair });
    expect(cookieFrom(forged)).toBeNull();
    expect(decodeURIComponent(forged.headers.get('location')!)).toContain('SAML response was rejected');

    const tampered = await samlResponse(p.id, { email: 'sam2@corp.example', name: 'Sam', groups: '' }, { mutate: (x) => x.replace(/sam2@corp\.example/g, 'admin@corp.example') });
    expect(cookieFrom(tampered)).toBeNull();

    const unsolicited = await samlResponse(p.id, { email: 'sam3@corp.example', name: 'Sam', groups: '' }, { inResponseTo: '_never-issued' });
    expect(cookieFrom(unsolicited)).toBeNull();
    expect(findUserByEmail('admin@corp.example')).toBeNull();
    expect(findUserByEmail('eve@corp.example')).toBeNull();
  });
});
