import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify } from 'jose';

const PUBLIC_PATHS = ['/login', '/api/auth/login', '/api/auth/logout', '/api/health'];

export async function middleware(request: NextRequest) {
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
    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET || 'ufop-admin-secret-change-me');
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
