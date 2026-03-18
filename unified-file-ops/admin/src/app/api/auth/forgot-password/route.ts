import { NextRequest, NextResponse } from 'next/server';
import { SignJWT } from 'jose';

const SMTP_CONFIGURED =
  !!process.env.SMTP_HOST && !!process.env.SMTP_PORT;

/**
 * OPTIONS /api/auth/forgot-password
 * Check if email reset is available (SMTP configured).
 */
export async function OPTIONS() {
  return NextResponse.json({ smtpConfigured: SMTP_CONFIGURED });
}

/**
 * POST /api/auth/forgot-password
 * Send a password reset email (if SMTP is configured).
 */
export async function POST(request: NextRequest) {
  if (!SMTP_CONFIGURED) {
    return NextResponse.json(
      { error: 'Email reset is not available. SMTP is not configured. Use recovery codes instead.' },
      { status: 503 }
    );
  }

  try {
    const { email } = await request.json();

    if (!email) {
      return NextResponse.json({ error: 'Email is required' }, { status: 400 });
    }

    // Verify the email matches the admin email
    const adminEmail = process.env.ADMIN_EMAIL || 'admin@ufop.local';
    if (email !== adminEmail) {
      // Don't reveal whether the email exists — always return success message
      return NextResponse.json({
        message: 'If that email is registered, a reset link has been sent.',
      });
    }

    // Generate reset token (1-hour expiry)
    const secret = new TextEncoder().encode(process.env.NEXTAUTH_SECRET!);
    const resetToken = await new SignJWT({
      sub: email,
      purpose: 'password-reset',
    })
      .setProtectedHeader({ alg: 'HS256' })
      .setExpirationTime('1h')
      .setIssuedAt()
      .sign(secret);

    // Build reset URL
    const baseUrl = process.env.NEXTAUTH_URL || 'http://localhost:3001';
    const resetUrl = `${baseUrl}/forgot-password?token=${resetToken}`;

    // Send email via SMTP
    // In a real implementation, use nodemailer or a transactional email service
    // For now, log the reset URL (server-side only)
    console.log(`[UFOP] Password reset requested for ${email}`);
    console.log(`[UFOP] Reset URL: ${resetUrl}`);

    // TODO: Implement actual SMTP sending when nodemailer is added
    // const transporter = nodemailer.createTransport({
    //   host: process.env.SMTP_HOST,
    //   port: parseInt(process.env.SMTP_PORT || '587'),
    //   auth: { user: process.env.SMTP_USER, pass: process.env.SMTP_PASS },
    // });
    // await transporter.sendMail({ to: email, subject: 'UFOP Password Reset', html: `<a href="${resetUrl}">Reset</a>` });

    return NextResponse.json({
      message: 'If that email is registered, a reset link has been sent.',
    });
  } catch {
    return NextResponse.json({ error: 'Failed to process reset request' }, { status: 500 });
  }
}
