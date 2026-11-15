// @vitest-environment jsdom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { cleanup, render, screen, waitFor } from '@testing-library/react';
import { AdminShell } from '@/components/layout/admin-shell';

const state = vi.hoisted(() => ({
  path: '/support', authenticated: false, replace: vi.fn(), validate: vi.fn(),
  applyTheme: vi.fn(), loadBranding: vi.fn(),
}));
vi.mock('next/navigation', () => ({ usePathname: () => state.path, useRouter: () => ({ replace: state.replace }) }));
vi.mock('@/lib/stores/auth-store', () => ({ useAuthStore: (selector?: (s: unknown) => unknown) => {
  const auth = { isAuthenticated: state.authenticated, isValidating: false, validateSession: state.validate, user: null };
  return selector ? selector(auth) : auth;
} }));
vi.mock('@/lib/stores/admin-store', () => ({ useAdminStore: (selector: (s: unknown) => unknown) => selector({ applyTheme: state.applyTheme, theme: 'light', sidebarMobileOpen: false }) }));
vi.mock('@/lib/stores/branding-store', () => ({ applyBrandColor: vi.fn(), useBrandingStore: (selector: (s: unknown) => unknown) => selector({ load: state.loadBranding }) }));
vi.mock('@/lib/i18n', () => ({ useTranslate: () => (key: string) => key }));
vi.mock('@/components/layout/sidebar', () => ({ Sidebar: () => null }));
vi.mock('@/components/layout/header', () => ({ Header: () => null }));
vi.mock('@/components/layout/view-as-banner', () => ({ ViewAsBanner: () => null }));
vi.mock('@/components/layout/footer', () => ({ Footer: () => null }));
vi.mock('@/components/ui/toaster', () => ({ Toaster: () => null }));
vi.mock('@/components/auth/reauth-dialog', () => ({ ReauthDialog: () => null }));

beforeEach(() => { state.path = '/support'; state.authenticated = false; state.replace.mockReset(); state.validate.mockResolvedValue(undefined); });
afterEach(cleanup);

it('hides a protected page before validation and redirects a signed-out visitor', async () => {
  render(<AdminShell><div>Private report</div></AdminShell>);
  expect(screen.queryByText('Private report')).toBeNull();
  await waitFor(() => expect(state.replace).toHaveBeenCalledWith('/login?redirect=%2Fsupport'));
  expect(screen.queryByText('Private report')).toBeNull();
});

it('unmounts protected content immediately when the user signs out', async () => {
  state.authenticated = true;
  const view = render(<AdminShell><div>Private report</div></AdminShell>);
  await waitFor(() => expect(screen.getByText('Private report')).toBeTruthy());
  state.authenticated = false;
  view.rerender(<AdminShell><div>Private report</div></AdminShell>);
  expect(screen.queryByText('Private report')).toBeNull();
  await waitFor(() => expect(state.replace).toHaveBeenCalled());
});

it('keeps password-reset and invitation pages public', async () => {
  state.path = '/reset-password';
  const view = render(<AdminShell><main>Reset form</main></AdminShell>);
  expect(screen.getByText('Reset form')).toBeTruthy();
  await waitFor(() => expect(state.validate).toHaveBeenCalled());
  state.path = '/invite/example';
  view.rerender(<AdminShell><main>Invitation form</main></AdminShell>);
  expect(screen.getByText('Invitation form')).toBeTruthy();
  expect(state.replace).not.toHaveBeenCalled();
});
