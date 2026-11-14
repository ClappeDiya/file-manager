import type { NextRequest } from 'next/server';
import { notAvailable } from '@/lib/features';

export async function GET(_request: NextRequest) { return notAvailable('connectors'); }
export async function POST(_request: NextRequest) { return notAvailable('connectors'); }
export async function PATCH(_request: NextRequest) { return notAvailable('connectors'); }
export async function DELETE(_request: NextRequest) { return notAvailable('connectors'); }
