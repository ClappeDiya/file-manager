import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import PrivacyPage from '@/app/legal/privacy/page';

describe('published privacy scope', () => {
  it('discloses the analytics observed in proxy-injected production pages', () => {
    const html = renderToStaticMarkup(<PrivacyPage />);
    const website = html.split('What our website collects')[1].split('<h2')[0];
    expect(website).toContain('Matomo');
    expect(website).toContain('/mtm/');
    expect(website).toMatch(/cookies/);
    expect(website).not.toMatch(/does not load analytics|no analytics cookies/i);
  });
  it('qualifies desktop network and managed-console data instead of claiming credentials never reach a server', () => {
    const text = renderToStaticMarkup(<PrivacyPage />).replace(/<[^>]+>/g, ' ');
    expect(text).toMatch(/Optional network features/);
    expect(text).toMatch(/stores an encrypted\s+copy/);
    expect(text).toMatch(/device enrollment/);
    expect(text).not.toMatch(/credentials never|Once Business launches|delete it at any time/i);
  });
});
