import { NextRequest, NextResponse } from 'next/server';
import * as db from '@/lib/db';

/** GET /api/devices - List device health status */
export async function GET(request: NextRequest) {
  const { searchParams } = new URL(request.url);
  const status = searchParams.get('status');
  const userId = searchParams.get('userId');
  const compliant = searchParams.get('compliant');

  let devices = db.getDevices();

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
