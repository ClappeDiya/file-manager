import type { NextRequest } from 'next/server';
import { notAvailable } from '@/lib/features';

// No process-memory connectors or synthetic connectivity results. A real
// implementation must persist definitions and probe the configured endpoint.
export async function GET(_request: NextRequest) { return notAvailable('connectors'); }
export async function POST(_request: NextRequest) { return notAvailable('connectors'); }
export async function DELETE(_request: NextRequest) { return notAvailable('connectors'); }
