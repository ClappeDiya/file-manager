import { NextRequest, NextResponse } from 'next/server';
import { jwtVerify } from 'jose';

const PUBLIC_PATHS = ['/login', '/api/auth/login', '/api/auth/logout', '/api/health'];

export async function middleware(request: NextRequest) {
  const { pathname } = request.nextUrl;

  // Allow public paths
  if (PUBLIC_PATHS.some((p) => pathname.startsWith(p))) {
    return NextResponse.next();
  }

  // Allow static assets and _next (match known extensions, not arbitrary dots)
  const STATIC_EXT = /\.(ico|png|jpg|jpeg|gif|svg|css|js|woff2?|ttf|eot|map|webp|avif)$/;
  if (pathname.startsWith('/_next') || pathname.startsWith('/favicon') || STATIC_EXT.test(pathname)) {
    return NextResponse.next();
  }

  // Security: CSRF protection for mutating API requests
  if (pathname.startsWith('/api/') && request.method !== 'GET') {
    const origin = request.headers.get('origin');
    const host = request.headers.get('host');

    if (!origin) {
      const ip = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim() || 'unknown';
      console.error(JSON.stringify({ level: 'warn', event: 'security.csrf_blocked', timestamp: new Date().toISOString(), ip, pathname, reason: 'missing_origin' }));
      return NextResponse.json({ error: 'CSRF validation failed: Origin header required' }, { status: 403 });
    }

    if (host) {
      try {
        const originHost = new URL(origin).host;
        if (originHost !== host) {
          const ip = request.headers.get('x-forwarded-for')?.split(',')[0]?.trim() || 'unknown';
          console.error(JSON.stringify({ level: 'warn', event: 'security.csrf_blocked', timestamp: new Date().toISOString(), ip, pathname, origin, host, reason: 'origin_mismatch' }));
          return NextResponse.json({ error: 'CSRF validation failed' }, { status: 403 });
        }
      } catch {
        return NextResponse.json({ error: 'CSRF validation failed: invalid Origin' }, { status: 403 });
      }
    }
  }

  // Check session cookie
  const sessionToken = request.cookies.get('ufop-admin-session')?.value;
  if (!sessionToken) {
    if (pathname.startsWith('/api/')) {
      return NextResponse.json({ error: 'Unauthorized' }, { status: 401 });
    }
    return NextResponse.redirect(new URL('/login', request.url));
  }

  // Security: fail if NEXTAUTH_SECRET is not configured
  const jwtSecret = process.env.NEXTAUTH_SECRET;
  if (!jwtSecret) {
    if (pathname.startsWith('/api/')) {
      return NextResponse.json({ error: 'Server configuration error' }, { status: 500 });
    }
    return NextResponse.redirect(new URL('/login', request.url));
  }

  // Verify JWT
  try {
    const secret = new TextEncoder().encode(jwtSecret);
    const { payload } = await jwtVerify(sessionToken, secret);

    // Add user info to request headers for downstream use
    const response = NextResponse.next();
    response.headers.set('x-user-email', String(payload.sub || ''));
    response.headers.set('x-user-role', String(payload.role || 'viewer'));
    return response;
  } catch {
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
