import { NextRequest, NextResponse } from 'next/server';
import { dataStore } from '@/lib/data-store';
import { requireRole, isAuthorized } from '@/lib/utils/require-role';

/** GET /api/devices - List device health status */
export async function GET(request: NextRequest) {
  const authResult = await requireRole(request, 'viewer');
  if (!isAuthorized(authResult)) return authResult;

  const { searchParams } = new URL(request.url);
  const status = searchParams.get('status');
  const userId = searchParams.get('userId');
  const compliant = searchParams.get('compliant');

  let devices = [...dataStore.devices];

  if (status) devices = devices.filter(d => d.status === status);
  if (userId) devices = devices.filter(d => d.userId === userId);
  if (compliant !== null) {
    devices = devices.filter(d => d.policyCompliance.isCompliant === (compliant === 'true'));
  }

  const stats = {
    total: devices.length,
    online: devices.filter(d => d.status === 'online').length,
    degraded: devices.filter(d => d.status === 'degraded').length,
    offline: devices.filter(d => d.status === 'offline').length,
    error: devices.filter(d => d.status === 'error').length,
    nonCompliant: devices.filter(d => !d.policyCompliance.isCompliant).length,
  };

  return NextResponse.json({ devices, stats });
}
