import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify } from 'jose';

const PUBLIC_PATHS = [
  '/login',
  '/forgot-password',
  '/api/auth/login',
  '/api/auth/logout',
  '/api/auth/reset-password',
  '/api/auth/forgot-password',
  '/api/health',
];

export async function proxy(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Allow public paths
  if (PUBLIC_PATHS.some((p) => pathname.startsWith(p))) {
    return NextResponse.next();
  }

  // Allow static assets and _next
  if (pathname.startsWith('/_next') || pathname.startsWith('/favicon') || pathname.includes('.')) {
    return NextResponse.next();
  }

  // Check session cookie
  const sessionToken = request.cookies.get('ufop-admin-session')?.value;
  if (!sessionToken) {
    // Redirect to login for page requests, return 401 for API
    if (pathname.startsWith('/api/')) {
      return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
    }
    return NextResponse.redirect(new URL('/login', request.url));
  }

  // Verify JWT
  try {
    if (!process.env.NEXTAUTH_SECRET) {
      console.error('[security] NEXTAUTH_SECRET environment variable is not set');
      if (pathname.startsWith('/api/')) {
        return NextResponse.json({ error: 'Server misconfigured' }, { status: 500 });
      }
      return NextResponse.redirect(new URL('/login', request.url));
    }
    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET);
    const { payload } = await jwtVerify(sessionToken, secret);

    // Add user info to request headers for downstream use
    const response = NextResponse.next();
    response.headers.set('x-user-email', String(payload.sub || ''));
    response.headers.set('x-user-role', String(payload.role || 'viewer'));
    return response;
  } catch {
    // Invalid/expired token - redirect to login
    if (pathname.startsWith('/api/')) {
      return NextResponse.json({ error: 'Session expired' }, { status: 401 });
    }
    const response = NextResponse.redirect(new URL('/login', request.url));
    response.cookies.delete('ufop-admin-session');
    return response;
  }
}

export const config = {
  matcher: ['/((?!_next/static|_next/image|favicon.ico).*)'],
};
