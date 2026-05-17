import { NextResponse } from 'next/server';

// Minimal waitlist endpoint. Stores submissions via a webhook to an external
// service (Resend, Formspree, Loops, etc.) configured via WAITLIST_WEBHOOK_URL.
// Until that env var is set the route returns 202 and logs to stdout, so the
// form on /pricing still works end-to-end during development.

export const runtime = 'nodejs';

type WaitlistBody = {
  email?: string;
  company?: string;
  seats?: string;
};

function isValidEmail(value: string): boolean {
  return /^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(value);
}

export async function POST(request: Request) {
  let body: WaitlistBody;
  try {
    body = await request.json();
  } catch {
    return NextResponse.json({ error: 'Invalid JSON.' }, { status: 400 });
  }

  const email = body.email?.trim();
  if (!email || !isValidEmail(email)) {
    return NextResponse.json({ error: 'A valid email is required.' }, { status: 400 });
  }

  const payload = {
    email,
    company: body.company?.trim() ?? '',
    seats: body.seats?.trim() ?? '',
    source: 'pricing-waitlist',
    timestamp: new Date().toISOString(),
  };

  const webhookUrl = process.env.WAITLIST_WEBHOOK_URL;
  if (webhookUrl) {
    const response = await fetch(webhookUrl, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(payload),
    });
    if (!response.ok) {
      console.error('Waitlist webhook failed', response.status, await response.text());
      return NextResponse.json({ error: 'Could not record your submission.' }, { status: 502 });
    }
  } else {
    console.log('Waitlist submission (no webhook configured):', payload);
  }

  return NextResponse.json({ ok: true }, { status: 202 });
}
