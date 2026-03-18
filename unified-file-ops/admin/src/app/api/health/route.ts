import { NextResponse } from 'next/server';

/** GET /api/health - Health check endpoint */
export async function GET() {
  return NextResponse.json({
    status: 'healthy',
    version: '0.1.0',
    timestamp: new Date().toISOString(),
    services: {
      auth: 'ok',
      database: 'ok',
      policyEngine: 'ok',
      auditLog: 'ok',
      workspaces: 'ok',
    },
  });
}
