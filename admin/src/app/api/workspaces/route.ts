import type { NextRequest } from 'next/server';
import { notAvailable } from '@/lib/features';

// The former route returned every organization's records and acknowledged
// grants/propagation without writing them. Refuse until authorized, scoped
// persistence and actual desktop delivery exist together.
export async function GET(_request: NextRequest) { return notAvailable('sharedWorkspaces'); }
export async function POST(_request: NextRequest) { return notAvailable('sharedWorkspaces'); }
