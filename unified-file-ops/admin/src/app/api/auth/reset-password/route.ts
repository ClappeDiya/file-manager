import { NextRequest, NextResponse } from 'next/server';
import * as crypto from 'crypto';
import * as fs from 'fs';
import * as path from 'path';
import { jwtVerify } from 'jose';

const DATA_DIR = path.join(process.cwd(), '.data');
const CODES_FILE = path.join(DATA_DIR, 'recovery-codes.json');
const PASSWORD_FILE = path.join(DATA_DIR, 'admin-password.json');

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

function hashPassword(password: string): string {
  const salt = crypto.randomBytes(16).toString('hex');
  const hash = crypto.pbkdf2Sync(password, salt, 100000, 64, 'sha512').toString('hex');
  return `${salt}:${hash}`;
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
    // Corrupted file
  }
  return null;
}

function saveCodes(data: CodesFile) {
  ensureDataDir();
  fs.writeFileSync(CODES_FILE, JSON.stringify(data, null, 2));
}

function savePassword(hashedPassword: string) {
  ensureDataDir();
  fs.writeFileSync(
    PASSWORD_FILE,
    JSON.stringify({ hash: hashedPassword, updated_at: new Date().toISOString() }, null, 2)
  );
}

/**
 * POST /api/auth/reset-password
 * Reset admin password using recovery code or email token.
 */
export async function POST(request: NextRequest) {
  try {
    const body = await request.json();
    const { method, recoveryCode, newPassword, token } = body;

    if (!newPassword || newPassword.length < 8) {
      return NextResponse.json(
        { error: 'New password must be at least 8 characters' },
        { status: 400 }
      );
    }

    if (method === 'code') {
      // Validate recovery code
      if (!recoveryCode) {
        return NextResponse.json({ error: 'Recovery code is required' }, { status: 400 });
      }

      const data = loadCodes();
      if (!data) {
        return NextResponse.json(
          { error: 'No recovery codes have been generated. Contact your server administrator.' },
          { status: 400 }
        );
      }

      const codeHash = hashCode(recoveryCode.trim().toUpperCase());
      const matchIndex = data.codes.findIndex((c) => c.hash === codeHash && !c.used);

      if (matchIndex === -1) {
        return NextResponse.json(
          { error: 'Invalid or already used recovery code' },
          { status: 401 }
        );
      }

      // Mark code as used
      data.codes[matchIndex].used = true;
      saveCodes(data);

      // Save new password
      savePassword(hashPassword(newPassword));

      const remaining = data.codes.filter((c) => !c.used).length;
      return NextResponse.json({
        success: true,
        remaining_codes: remaining,
        message: remaining <= 2
          ? `Password reset. Warning: only ${remaining} recovery code(s) remaining. Generate new codes after login.`
          : 'Password reset successfully.',
      });
    }

    if (method === 'token') {
      // Validate email reset token
      if (!token) {
        return NextResponse.json({ error: 'Reset token is required' }, { status: 400 });
      }

      try {
        const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET!);
        const { payload } = await jwtVerify(token, secret);

        if (payload.purpose !== 'password-reset') {
          return NextResponse.json({ error: 'Invalid reset token' }, { status: 401 });
        }

        // Save new password
        savePassword(hashPassword(newPassword));

        return NextResponse.json({
          success: true,
          message: 'Password reset successfully.',
        });
      } catch {
        return NextResponse.json(
          { error: 'Invalid or expired reset token' },
          { status: 401 }
        );
      }
    }

    return NextResponse.json({ error: 'Invalid reset method' }, { status: 400 });
  } catch {
    return NextResponse.json({ error: 'Password reset failed' }, { status: 500 });
  }
}
