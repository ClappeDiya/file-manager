/** In-memory failure counters (per process): `limit` failures per `windowMs` per key. */
export class FailureLimiter {
  private entries = new Map<string, { count: number; resetAt: number }>();
  constructor(private limit: number, private windowMs: number) {}

  /** Seconds until the key may try again (0 = allowed). */
  wait(key: string): number {
    const e = this.entries.get(key);
    if (!e || Date.now() > e.resetAt) return 0;
    return e.count >= this.limit ? Math.ceil((e.resetAt - Date.now()) / 1000) : 0;
  }

  fail(key: string): void {
    const e = this.entries.get(key);
    if (!e || Date.now() > e.resetAt) this.entries.set(key, { count: 1, resetAt: Date.now() + this.windowMs });
    else e.count++;
  }

  reset(): void {
    this.entries.clear();
  }
}

export const loginLimiter = new FailureLimiter(5, 15 * 60 * 1000);
/** Agent enrollment endpoints (registration tokens, claim codes). */
export const enrollLimiter = new FailureLimiter(20, 15 * 60 * 1000);
