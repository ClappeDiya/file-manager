import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import type { AdminUser, Role } from '@/lib/types/auth';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/** GET /api/users - List all users */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const role = searchParams.get('role');
  const search = searchParams.get('search');
  const active = searchParams.get('active');

  let result = [...dataStore.users];

  if (role) result = result.filter(u => u.role === role);
  if (active !== null) result = result.filter(u => u.isActive === (active === 'true'));
  if (search) {
    const q = search.toLowerCase();
    result = result.filter(u =>
      u.name.toLowerCase().includes(q) || u.email.toLowerCase().includes(q)
    );
  }

  return NextResponse.json({ users: result, total: result.length });
}

/** POST /api/users - Invite a new user */
export async function POST(request: NextRequest) {
  const authResult = await requireRole(request, 'admin');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { email, role, name } = body;

    if (!email) {
      return NextResponse.json({ error: 'Email is required' }, { status: 400 });
    }

    if (dataStore.users.some(u => u.email === email)) {
      return NextResponse.json({ error: 'User already exists' }, { status: 409 });
    }

    const newUser: AdminUser = {
      id: `usr_${Date.now()}`,
      email,
      name: name || email.split('@')[0],
      role: (role as Role) || 'user',
      permissions: [],
      organizationId: 'org_001',
      createdAt: new Date().toISOString(),
      isActive: true,
    };

    dataStore.users.push(newUser);
    return NextResponse.json({ user: newUser }, { status: 201 });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}

/** PATCH /api/users - Bulk operations */
export async function PATCH(request: NextRequest) {
  const authResult = await requireRole(request, 'admin');
  if (!isAuthorized(authResult)) return authResult;

  try {
    const body = await request.json();
    const { action, userIds } = body;

    if (!action || !Array.isArray(userIds)) {
      return NextResponse.json({ error: 'Action and userIds required' }, { status: 400 });
    }

    switch (action) {
      case 'deactivate':
        dataStore.users = dataStore.users.map(u => userIds.includes(u.id) ? { ...u, isActive: false } : u);
        break;
      case 'activate':
        dataStore.users = dataStore.users.map(u => userIds.includes(u.id) ? { ...u, isActive: true } : u);
        break;
      case 'remove':
        dataStore.users = dataStore.users.filter(u => !userIds.includes(u.id));
        break;
      default:
        return NextResponse.json({ error: 'Unknown action' }, { status: 400 });
    }

    return NextResponse.json({ success: true, affected: userIds.length });
  } catch {
    return NextResponse.json({ error: 'Invalid request' }, { status: 400 });
  }
}
