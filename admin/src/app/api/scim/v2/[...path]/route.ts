import { NextRequest } from 'next/server';
import { clientIp, readJson } from '@/lib/session';
import { scimTokenFor, type ScimToken } from '@/lib/db/repos/scim';
import { enrollLimiter } from '@/lib/rate-limit';
import { isEnabled } from '@/lib/features';
import { publicOrigin } from '@/lib/route-helpers';
import {
  createScimGroup,
  createScimUser,
  deleteScimGroup,
  deleteScimUser,
  getScimGroup,
  getScimUser,
  listScimGroups,
  listScimUsers,
  patchScimGroup,
  patchScimUser,
  replaceScimGroup,
  replaceScimUser,
  resourceTypes,
  schemas,
  ScimError,
  scimErrorResponse,
  scimJson,
  serviceProviderConfig,
} from '@/lib/scim';

/**
 * SCIM 2.0 endpoint: /api/scim/v2/{ServiceProviderConfig|ResourceTypes|Schemas|Users[/id]|Groups[/id]}
 * Authorization: Bearer ufop_scim_… (Users → Provisioning). The token decides the organization.
 */
type P = { params: Promise<{ path: string[] }> };

async function handle(request: NextRequest, { params }: P, method: string) {
  try {
    if (!isEnabled('scim')) throw new ScimError(501, 'SCIM provisioning is not available');
    const ip = clientIp(request);
    if (enrollLimiter.wait(`scim:${ip}`)) throw new ScimError(429, 'Too many failed requests');
    const h = request.headers.get('authorization') ?? '';
    const token: ScimToken | null = h.startsWith('Bearer ') ? scimTokenFor(h.slice(7).trim()) : null;
    if (!token) {
      enrollLimiter.fail(`scim:${ip}`);
      throw new ScimError(401, 'A valid SCIM bearer token is required');
    }
    const [resource, id, ...rest] = (await params).path;
    if (rest.length) throw new ScimError(404, 'Unknown endpoint');
    const base = `${publicOrigin(request)}/api/scim/v2`;
    const q = new URL(request.url).searchParams;
    const body = async () => {
      const b = await readJson<Record<string, unknown>>(request);
      if (!b || typeof b !== 'object' || Array.isArray(b)) throw new ScimError(400, 'A JSON object is required', 'invalidSyntax');
      return b;
    };

    if (method === 'GET' && !id) {
      if (resource === 'ServiceProviderConfig') return scimJson(serviceProviderConfig(base));
      if (resource === 'ResourceTypes') return scimJson(resourceTypes(base));
      if (resource === 'Schemas') return scimJson(schemas());
    }
    if (resource === 'Users') {
      if (method === 'GET') return scimJson(id ? getScimUser(token, id, base) : listScimUsers(token, q, base));
      if (method === 'POST' && !id) {
        const u = createScimUser(token, await body(), base);
        return scimJson(u, 201, { location: u.meta.location });
      }
      if (id && method === 'PUT') return scimJson(replaceScimUser(token, id, await body(), base));
      if (id && method === 'PATCH') return scimJson(patchScimUser(token, id, await body(), base));
      if (id && method === 'DELETE') {
        deleteScimUser(token, id);
        return scimJson(null, 204);
      }
    }
    if (resource === 'Groups') {
      if (method === 'GET') return scimJson(id ? getScimGroup(token, id, q, base) : listScimGroups(token, q, base));
      if (method === 'POST' && !id) {
        const g = createScimGroup(token, await body(), base);
        return scimJson(g, 201, { location: g.meta.location });
      }
      if (id && method === 'PUT') return scimJson(replaceScimGroup(token, id, await body(), base));
      if (id && method === 'PATCH') return scimJson(patchScimGroup(token, id, await body(), base));
      if (id && method === 'DELETE') {
        deleteScimGroup(token, id);
        return scimJson(null, 204);
      }
    }
    throw new ScimError(resource === 'Users' || resource === 'Groups' ? 405 : 404, resource === 'Users' || resource === 'Groups' ? `${method} is not supported here` : 'Unknown endpoint');
  } catch (e) {
    return scimErrorResponse(e);
  }
}

export const GET = (r: NextRequest, p: P) => handle(r, p, 'GET');
export const POST = (r: NextRequest, p: P) => handle(r, p, 'POST');
export const PUT = (r: NextRequest, p: P) => handle(r, p, 'PUT');
export const PATCH = (r: NextRequest, p: P) => handle(r, p, 'PATCH');
export const DELETE = (r: NextRequest, p: P) => handle(r, p, 'DELETE');
