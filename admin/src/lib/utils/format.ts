/**
 * Formatting utilities for the admin console.
 */

/** Format bytes into human-readable size */
export function formatBytes(bytes: number, decimals = 2): string {
  if (bytes === 0) return '0 B';
  const k = 1024;
  const sizes = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${parseFloat((bytes / Math.pow(k, i)).toFixed(decimals))} ${sizes[i]}`;
}

let displayTimeZone: string | undefined;
let displayLocale = 'en-US';

const REGION: Record<string, string> = { en: 'en-US', de: 'de-DE', es: 'es-ES', fr: 'fr-FR', ja: 'ja-JP', ko: 'ko-KR', pt: 'pt-BR', zh: 'zh-CN' };

/** Format dates and numbers for this console language (`en`, `de`, …). */
export function setDisplayLocale(locale: string): void {
  displayLocale = REGION[locale] ?? 'en-US';
}

/** Show dates in this IANA time zone (the signed-in user's preference); undefined = the browser's. */
export function setDisplayTimeZone(tz: string | null | undefined): void {
  displayTimeZone = tz || undefined;
}

export function getDisplayTimeZone(): string {
  return displayTimeZone ?? Intl.DateTimeFormat().resolvedOptions().timeZone;
}

/** Format a date for display */
export function formatDate(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  return d.toLocaleDateString(displayLocale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    timeZone: displayTimeZone,
  });
}

/** Format a datetime for display (with the zone's abbreviation when a preference is set) */
export function formatDateTime(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  return d.toLocaleString(displayLocale, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
    timeZone: displayTimeZone,
    ...(displayTimeZone ? { timeZoneName: 'short' as const } : {}),
  });
}

/** Format relative time (e.g., "2 hours ago") */
export function formatRelativeTime(date: string | Date): string {
  const d = typeof date === 'string' ? new Date(date) : date;
  const now = new Date();
  const diffMs = now.getTime() - d.getTime();
  const diffSec = Math.floor(diffMs / 1000);
  const diffMin = Math.floor(diffSec / 60);
  const diffHour = Math.floor(diffMin / 60);
  const diffDay = Math.floor(diffHour / 24);

  if (displayLocale !== 'en-US' && diffDay < 7) {
    const rtf = new Intl.RelativeTimeFormat(displayLocale, { numeric: 'auto', style: 'short' });
    if (diffSec < 60) return rtf.format(0, 'second');
    if (diffMin < 60) return rtf.format(-diffMin, 'minute');
    if (diffHour < 24) return rtf.format(-diffHour, 'hour');
    return rtf.format(-diffDay, 'day');
  }
  if (diffSec < 60) return 'just now';
  if (diffMin < 60) return `${diffMin}m ago`;
  if (diffHour < 24) return `${diffHour}h ago`;
  if (diffDay < 7) return `${diffDay}d ago`;
  return formatDate(d);
}

/** Format a number with commas */
export function formatNumber(n: number): string {
  return n.toLocaleString(displayLocale);
}

/** Format a duration in minutes */
export function formatDuration(minutes: number): string {
  if (minutes < 1) return 'less than 1 min';
  if (minutes < 60) return `${Math.round(minutes)} min`;
  const hours = Math.floor(minutes / 60);
  const mins = Math.round(minutes % 60);
  if (mins === 0) return `${hours}h`;
  return `${hours}h ${mins}m`;
}

/** Truncate string with ellipsis */
export function truncate(str: string, maxLength: number): string {
  if (str.length <= maxLength) return str;
  return str.slice(0, maxLength - 3) + '...';
}

/** Generate SHA-256 hash (for audit integrity) */
export async function generateHash(content: string): Promise<string> {
  const encoder = new TextEncoder();
  const data = encoder.encode(content);
  const hashBuffer = await crypto.subtle.digest('SHA-256', data);
  const hashArray = Array.from(new Uint8Array(hashBuffer));
  return hashArray.map(b => b.toString(16).padStart(2, '0')).join('');
}
