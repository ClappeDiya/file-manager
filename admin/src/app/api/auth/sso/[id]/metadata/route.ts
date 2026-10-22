import { NextRequest, NextResponse } from 'next/server';
import { providerOrThrow, samlMetadata } from '@/lib/sso';
import { orgErrorResponse, publicOrigin } from '@/lib/route-helpers';

/** GET /api/auth/sso/:id/metadata — SAML service provider metadata for the IdP admin. */
export async function GET(request: NextRequest, { params }: { params: Promise<{ id: string }> }) {
  try {
    const p = providerOrThrow((await params).id);
    if (p.kind !== 'saml') return NextResponse.json({ error: 'Metadata is only published for SAML providers' }, { status: 404 });
    return new NextResponse(samlMetadata(p, publicOrigin(request)), { headers: { 'content-type': 'application/samlmetadata+xml' } });
  } catch (e) {
    return orgErrorResponse(e);
  }
}
