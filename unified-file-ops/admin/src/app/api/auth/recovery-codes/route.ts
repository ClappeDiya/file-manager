import { NextRequest, NextResponse } from 'next/server';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';

const DATA_DIR = path.join(process.cwd(), '.data');
const CODES_FILE = path.join(DATA_DIR, 'recovery-codes.json');

interface StoredCode {
  hash: string;
  used: boolean;
  created_at: string;
}

interface CodesFile {
  codes: StoredCode[];
  created_at: string;
}

function hashCode(code: string): string {
  return crypto.createHash('sha256').update(code).digest('hex');
}

function generateCode(): string {
  const chars = 'ABCDEFGHJKLMNPQRSTUVWXYZ23456789';
  const segments = [];
  for (let s = 0; s < 3; s++) {
    let segment = '';
    for (let i = 0; i < 4; i++) {
      segment += chars[crypto.randomInt(chars.length)];
    }
    segments.push(segment);
  }
  return segments.join('-');
}

function ensureDataDir() {
  if (!fs.existsSync(DATA_DIR)) {
    fs.mkdirSync(DATA_DIR, { recursive: true });
  }
}

function loadCodes(): CodesFile | null {
  try {
    if (fs.existsSync(CODES_FILE)) {
      return JSON.parse(fs.readFileSync(CODES_FILE, 'utf-8'));
    }
  } catch {
    // Corrupted file — treat as no codes
  }
  return null;
}

function saveCodes(data: CodesFile) {
  ensureDataDir();
  fs.writeFileSync(CODES_FILE, JSON.stringify(data, null, 2));
}

/**
 * POST /api/auth/recovery-codes
 * Generate new recovery codes. Requires active session.
 */
export async function POST(request: NextRequest) {
  // Verify session (cookie check — middleware handles auth for non-public paths)
  const sessionToken = request.cookies.get('ufop-admin-session')?.value;
  if (!sessionToken) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const COUNT = 8;
  const plainCodes: string[] = [];
  const storedCodes: StoredCode[] = [];

  for (let i = 0; i < COUNT; i++) {
    const code = generateCode();
    plainCodes.push(code);
    storedCodes.push({
      hash: hashCode(code),
      used: false,
      created_at: new Date().toISOString(),
    });
  }

  saveCodes({
    codes: storedCodes,
    created_at: new Date().toISOString(),
  });

  return NextResponse.json({
    codes: plainCodes,
    message: 'Save these codes somewhere safe. Each can only be used once.',
  });
}

/**
 * GET /api/auth/recovery-codes
 * Get recovery code status (how many remaining). Requires active session.
 */
export async function GET(request: NextRequest) {
  const sessionToken = request.cookies.get('ufop-admin-session')?.value;
  if (!sessionToken) {
    return NextResponse.json({ error: 'Authentication required' }, { status: 401 });
  }

  const data = loadCodes();
  if (!data) {
    return NextResponse.json({ generated: false, total: 0, remaining: 0 });
  }

  const remaining = data.codes.filter((c) => !c.used).length;
  return NextResponse.json({
    generated: true,
    total: data.codes.length,
    remaining,
    created_at: data.created_at,
  });
}
