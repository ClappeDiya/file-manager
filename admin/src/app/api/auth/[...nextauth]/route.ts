/**
 * Auth providers discovery endpoint.
 *
 * Returns the available authentication methods. The actual auth flow
 * is handled by /api/auth/login (JWT-based with bcrypt password verification).
 *
 * GET /api/auth/providers — list available auth methods
 * POST /api/auth/providers — redirects to /api/auth/login
 */

import { NextRequest, NextResponse } from 'next/server';

export async function GET() {
  return NextResponse.json({
    providers: [
      { id: 'credentials', name: 'Email & Password', type: 'credentials', active: true },
      { id: 'saml', name: 'SAML 2.0 SSO', type: 'saml', active: false, tier: 'business' },
    ],
    loginEndpoint: '/api/auth/login',
    logoutEndpoint: '/api/auth/logout',
    sessionEndpoint: '/api/auth/session',
  });
}

export async function POST(request: NextRequest) {
  // Redirect to the real login endpoint using the request's own origin
  return NextResponse.redirect(new URL('/api/auth/login', request.url), 307);
}
