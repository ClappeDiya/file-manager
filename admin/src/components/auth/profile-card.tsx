'use client';

/** My account → Profile: display name and the time zone the console shows times in. */
import React, { useMemo, useState } from 'react';
import { Card, CardDescription, CardHeader, CardTitle } from '@/components/ui/card';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Select } from '@/components/ui/select';
import { ErrorBanner } from '@/components/fleet/badges';
import { api, errorText } from '@/lib/api-client';
import { useAuthStore } from '@/lib/stores/auth-store';
import { useToastStore } from '@/lib/stores/toast-store';
import { formatDateTime } from '@/lib/utils/format';
import type { AdminUser } from '@/lib/types/auth';
import { LOCALE_NAMES, SUPPORTED_LOCALES, useTranslate, useI18nStore, type Locale } from '@/lib/i18n';

const zones = (): string[] => {
  try {
    return (Intl as unknown as { supportedValuesOf(k: string): string[] }).supportedValuesOf('timeZone');
  } catch {
    return ['UTC'];
  }
};

export function ProfileCard() {
  const user = useAuthStore((s) => s.user);
  const setUser = useAuthStore((s) => s.setUser);
  const notify = useToastStore((s) => s.notify);
  const [name, setName] = useState<string | null>(null);
  const [tz, setTz] = useState<string | null>(null);
  const [locale, setLocale] = useState<string | null>(null);
  const t = useTranslate();
  const setUiLocale = useI18nStore((s) => s.setLocale);
  const [err, setErr] = useState<string | null>(null);
  // The zone list is long and fixed; only the "browser" label follows the language.
  const zoneOptions = useMemo(() => ['UTC', ...zones().filter((z) => z !== 'UTC')].map((z) => ({ value: z, label: z.replace(/_/g, ' ') })), []);
  const options = [{ value: '', label: t('account.browserZone', { zone: Intl.DateTimeFormat().resolvedOptions().timeZone }) }, ...zoneOptions];
  if (!user) return null;
  const curTz = tz ?? user.timeZone ?? '';
  const curLocale = locale ?? user.locale ?? '';
  const save = async () => {
    setErr(null);
    try {
      const r = await api<{ user: AdminUser }>('/api/account/profile', {
        method: 'PATCH',
        body: { ...(name !== null ? { name } : {}), ...(tz !== null ? { timeZone: tz || null } : {}), ...(locale !== null ? { locale: locale || null } : {}) },
      });
      setUser({ ...user, ...r.user, role: user.role });
      if (locale) setUiLocale(locale as Locale);
      setName(null);
      setTz(null);
      setLocale(null);
      notify(t('account.saved'), 'success');
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <Card>
      <CardHeader className="flex-col items-start gap-1">
        <CardTitle>{t('account.profile')}</CardTitle>
        <CardDescription>{t('account.profileDescription', { now: formatDateTime(new Date()) })}</CardDescription>
      </CardHeader>
      <ErrorBanner message={err} />
      <div className="space-y-2">
        <Input label={t('account.name')} value={name ?? user.name} onChange={(e) => setName(e.target.value)} />
        <Select label={t('account.timeZone')} options={options} value={curTz} onChange={(e) => setTz(e.target.value)} />
        <Select
          label={t('common.language')}
          options={[{ value: '', label: t('account.browserLanguage') }, ...SUPPORTED_LOCALES.map((l) => ({ value: l, label: LOCALE_NAMES[l] }))]}
          value={curLocale}
          onChange={(e) => setLocale(e.target.value)}
        />
        <Button size="sm" disabled={name === null && tz === null && locale === null} onClick={() => void save()}>
          {t('common.save')}
        </Button>
      </div>
    </Card>
  );
}
