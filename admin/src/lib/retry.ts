/**
 * One retry schedule for outbound deliveries (webhooks, the notification
 * outbox, SIEM forwarding): attempt n+1 waits BACKOFF_SECONDS[n-1].
 */

/** Seconds before attempt n+1 (after n failed attempts). */
export const BACKOFF_SECONDS = [60, 300, 1800, 7200, 21600];
/** Queued deliveries give up after this many attempts. */
export const MAX_ATTEMPTS = BACKOFF_SECONDS.length + 1;

/** The wait after `failures` consecutive failures, capped at the longest step (for senders that never give up). */
export function backoffSeconds(failures: number): number {
  if (failures <= 0) return 0;
  return BACKOFF_SECONDS[Math.min(failures, BACKOFF_SECONDS.length) - 1];
}
