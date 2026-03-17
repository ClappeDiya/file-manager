/**
 * NextAuth.js route handler placeholder.
 *
 * In production, this would be configured with:
 * - CredentialsProvider for username/password auth
 * - SAML provider for SSO (Business tier)
 * - SCIM integration for user provisioning (Enterprise tier)
 *
 * For now, auth is handled by the custom /api/auth/login route
 * and the client-side auth store.
 */

import { NextResponse } from 'next/server';

export async function GET() {
  return NextResponse.json({
    providers: [
      { id: 'credentials', name: 'Email & Password', type: 'credentials' },
      { id: 'saml', name: 'SAML 2.0 SSO', type: 'saml', tier: 'business' },
      { id: 'scim', name: 'SCIM Provisioning', type: 'scim', tier: 'enterprise' },
    ],
  });
}

export async function POST() {
  return NextResponse.json({ error: 'Use /api/auth/login for authentication' }, { status: 307 });
}
