import { NextRequest, NextResponse } from 'next/server';
import { providersForDomain } from '@/lib/db/repos/sso';

/** GET /api/auth/sso/discover?email= — sign-in methods for the address's domain. */
export async function GET(request: NextRequest) {
  const email = request.nextUrl.searchParams.get('email')?.trim().toLowerCase() ?? '';
  const domain = email.includes('@') ? email.split('@').pop()! : '';
  if (!domain) return NextResponse.json({ providers: [] });
  return NextResponse.json({ providers: providersForDomain(domain).map((p) => ({ id: p.id, name: p.name, kind: p.kind })) });
}
