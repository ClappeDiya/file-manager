import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify } from 'jose';

export type UserRole = 'admin' | 'manager' | 'viewer';

interface SessionUser {
  email: string;
  name: string;
  role: UserRole;
}

const ROLE_HIERARCHY: Record<UserRole, number> = {
  admin: 3,
  manager: 2,
  viewer: 1,
};

export async function requireRole(
  request: NextRequest,
  minimumRole: UserRole
): Promise<{ user: SessionUser } | NextResponse> {
  const jwtSecret = process.env.NEXTAUTH_SECRET;
  if (!jwtSecret) {
    return NextResponse.json({ error: 'Server configuration error' }, { status: 500 });
  }

  const sessionToken = request.cookies.get('ufop-admin-session')?.value;
  if (!sessionToken) {
    return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
  }

  try {
    const secret = new TextEncoder().encode(jwtSecret);
    const { payload } = await jwtVerify(sessionToken, secret);
    const userRole = (payload.role as UserRole) || 'viewer';

    if (ROLE_HIERARCHY[userRole] < ROLE_HIERARCHY[minimumRole]) {
      return NextResponse.json({ error: 'Forbidden: insufficient role' }, { status: 403 });
    }

    return {
      user: {
        email: String(payload.sub || ''),
        name: String(payload.name || ''),
        role: userRole,
      },
    };
  } catch {
    return NextResponse.json({ error: 'Session expired' }, { status: 401 });
  }
}

// Helper to check if the result is a NextResponse (error) or a user object
export function isAuthorized(result: { user: SessionUser } | NextResponse): result is { user: SessionUser } {
  return 'user' in result;
}
