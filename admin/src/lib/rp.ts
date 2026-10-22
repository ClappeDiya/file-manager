/** The WebAuthn relying party: the console's public origin and its host name. */
import { publicOrigin } from './route-helpers';

export function relyingParty(req: Request): { id: string; name: string; origin: string } {
  const origin = publicOrigin(req);
  return { id: new URL(origin).hostname, name: 'UFOP Console', origin };
}
