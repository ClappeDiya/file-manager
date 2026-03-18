import { cookies } from 'next/headers';
import { jwtVerify } from 'jose';

export interface SessionUser {
  email: string;
  name: string;
  role: 'admin' | 'manager' | 'viewer';
}

export async function getSession(): Promise<SessionUser | null> {
  try {
    const cookieStore = await cookies();
    const token = cookieStore.get('ufop-admin-session')?.value;
    if (!token) return null;

    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET!);
    const { payload } = await jwtVerify(token, secret);

    return {
      email: String(payload.sub || ''),
      name: String(payload.name || ''),
      role: (payload.role as SessionUser['role']) || 'viewer',
    };
  } catch {
    return null;
  }
}

export function hasPermission(role: SessionUser['role'], requiredRole: 'admin' | 'manager' | 'viewer'): boolean {
  const hierarchy = { admin: 3, manager: 2, viewer: 1 };
  return hierarchy[role] >= hierarchy[requiredRole];
}
