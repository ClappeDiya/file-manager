import { renderToStaticMarkup } from 'react-dom/server';
import { expect, it } from 'vitest';
import ChangelogPage from '@/app/changelog/page';

it('publishes one page title while retaining the real release sections', async () => {
  const html = renderToStaticMarkup(await ChangelogPage());
  expect(html.match(/<h1[ >]/g)).toHaveLength(1);
  expect(html).toContain('Changelog');
  expect(html).toContain('Unreleased');
  expect(html).toMatch(/<h2[^>]*>/);
});
