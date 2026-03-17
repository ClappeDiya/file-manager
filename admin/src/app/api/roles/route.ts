import { NextRequest, NextResponse } from 'next/server';
import { ROLE_LABELS, ROLE_PERMISSIONS, ROLE_HIERARCHY, type Role } from '@/lib/types/auth';

/** GET /api/roles - List all roles and their permissions */
export async function GET() {
  const roles = (Object.entries(ROLE_LABELS) as [Role, string][]).map(([role, label]) => ({
    id: role,
    label,
    hierarchy: ROLE_HIERARCHY[role],
    permissions: ROLE_PERMISSIONS[role],
    permissionCount: ROLE_PERMISSIONS[role].length,
  }));

  return NextResponse.json({ roles });
}
