import { afterEach, beforeEach } from 'vitest';
import { resetDb } from '@/lib/db';
import { loginLimiter, enrollLimiter } from '@/lib/rate-limit';

process.env.ADMIN_DB_PATH = ':memory:';
process.env.NEXTAUTH_SECRET = 'test-secret-for-the-admin-console-0123456789';
delete process.env.ADMIN_ENCRYPTION_KEY;

beforeEach(() => {
  resetDb();
  loginLimiter.reset();
  enrollLimiter.reset();
});

afterEach(() => resetDb());
