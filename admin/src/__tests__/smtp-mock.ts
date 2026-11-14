/**
 * A small in-process SMTP server for tests: EHLO/HELO, STARTTLS (with the
 * given certificate), AUTH PLAIN and LOGIN (one user), MAIL, RCPT, DATA,
 * RSET, NOOP and QUIT. Accepted messages are kept in memory, each prefixed
 * with `X-Rcpt: <recipient>` lines, like the mail emulator the tests used to
 * need. Mail is refused before STARTTLS and AUTH, as a submission server does.
 */
import net from 'net';
import tls from 'tls';
import type { AddressInfo } from 'net';

export interface SmtpMock {
  port: number;
  /** Raw accepted messages, oldest first. */
  messages: string[];
  close: () => Promise<void>;
}

export interface SmtpMockOptions {
  cert: string;
  key: string;
  user: string;
  pass: string;
}

export async function startSmtpMock(o: SmtpMockOptions): Promise<SmtpMock> {
  const messages: string[] = [];
  const sockets = new Set<net.Socket>();
  const secureContext = tls.createSecureContext({ cert: o.cert, key: o.key });

  const server = net.createServer((raw) => {
    sockets.add(raw);
    raw.on('close', () => sockets.delete(raw));
    let sock: net.Socket = raw;
    let buf = '';
    let secure = false;
    let authed = false;
    let from: string | null = null;
    let rcpts: string[] = [];
    let data: string[] | null = null;
    let authStep: null | 'user' | 'pass' | 'plain' = null;
    let loginUser = '';

    const say = (line: string) => sock.write(`${line}\r\n`);
    const ehlo = () => {
      const ext = secure ? ['AUTH PLAIN LOGIN'] : ['STARTTLS'];
      const lines = ['mock.localhost', '8BITMIME', 'SIZE 10485760', ...ext];
      lines.forEach((l, i) => say(`250${i === lines.length - 1 ? ' ' : '-'}${l}`));
    };
    const checkCreds = (user: string, pass: string) => {
      authStep = null;
      if (user === o.user && pass === o.pass) {
        authed = true;
        say('235 2.7.0 Authentication successful');
      } else {
        say('535 5.7.8 Authentication credentials invalid');
      }
    };

    const onLine = (line: string) => {
      if (data) {
        if (line === '.') {
          const body = data.join('\r\n');
          messages.push(`${rcpts.map((r) => `X-Rcpt: ${r}`).join('\r\n')}\r\n${body}\r\n`);
          data = null;
          from = null;
          rcpts = [];
          say('250 2.0.0 Ok: queued');
        } else {
          data.push(line.startsWith('..') ? line.slice(1) : line);
        }
        return;
      }
      if (authStep) {
        const v = Buffer.from(line.trim(), 'base64').toString('utf8');
        if (line.trim() === '*') {
          authStep = null;
          return say('501 5.7.0 Authentication cancelled');
        }
        if (authStep === 'plain') {
          const [, user = '', pass = ''] = v.split('\u0000');
          return checkCreds(user, pass);
        }
        if (authStep === 'user') {
          loginUser = v;
          authStep = 'pass';
          return say(`334 ${Buffer.from('Password:').toString('base64')}`);
        }
        return checkCreds(loginUser, v);
      }
      const [verb = '', ...rest] = line.split(' ');
      const arg = rest.join(' ');
      switch (verb.toUpperCase()) {
        case 'EHLO':
          return ehlo();
        case 'HELO':
          return say('250 mock.localhost');
        case 'STARTTLS': {
          if (secure) return say('503 5.5.1 TLS already active');
          say('220 2.0.0 Ready to start TLS');
          const plain = sock;
          plain.removeAllListeners('data');
          const t = new tls.TLSSocket(plain, { isServer: true, secureContext });
          t.on('data', onData);
          t.on('error', () => plain.destroy());
          sock = t;
          secure = true;
          buf = '';
          return;
        }
        case 'AUTH': {
          if (!secure) return say('538 5.7.11 Encryption required for requested authentication mechanism');
          const [mech = '', initial] = arg.split(' ');
          if (mech.toUpperCase() === 'PLAIN') {
            if (initial) {
              const [, user = '', pass = ''] = Buffer.from(initial, 'base64').toString('utf8').split('\u0000');
              return checkCreds(user, pass);
            }
            authStep = 'plain';
            return say('334 ');
          }
          if (mech.toUpperCase() === 'LOGIN') {
            if (initial) {
              loginUser = Buffer.from(initial, 'base64').toString('utf8');
              authStep = 'pass';
              return say(`334 ${Buffer.from('Password:').toString('base64')}`);
            }
            authStep = 'user';
            return say(`334 ${Buffer.from('Username:').toString('base64')}`);
          }
          return say('504 5.5.4 Unrecognized authentication type');
        }
        case 'MAIL':
          if (!authed) return say('530 5.7.0 Authentication required');
          from = arg.replace(/^FROM:\s*/i, '');
          rcpts = [];
          return say('250 2.1.0 Ok');
        case 'RCPT': {
          if (!from) return say('503 5.5.1 Need MAIL command');
          const m = /^TO:\s*<([^>]*)>/i.exec(arg);
          if (!m) return say('501 5.1.3 Bad recipient address syntax');
          rcpts.push(m[1]);
          return say('250 2.1.5 Ok');
        }
        case 'DATA':
          if (!rcpts.length) return say('503 5.5.1 Need RCPT command');
          data = [];
          return say('354 End data with <CR><LF>.<CR><LF>');
        case 'RSET':
          from = null;
          rcpts = [];
          return say('250 2.0.0 Ok');
        case 'NOOP':
          return say('250 2.0.0 Ok');
        case 'QUIT':
          say('221 2.0.0 Bye');
          return sock.end();
        default:
          return say('502 5.5.2 Command not recognized');
      }
    };

    function onData(chunk: Buffer) {
      buf += chunk.toString('utf8');
      let i: number;
      while ((i = buf.indexOf('\r\n')) >= 0) {
        const line = buf.slice(0, i);
        buf = buf.slice(i + 2);
        const before = sock;
        onLine(line);
        // After STARTTLS the rest of this chunk (none, per RFC 3207) is discarded.
        if (sock !== before) return;
      }
    }

    raw.on('data', onData);
    raw.on('error', () => undefined);
    say('220 mock.localhost ESMTP ready');
  });

  await new Promise<void>((r) => server.listen(0, '127.0.0.1', r));
  return {
    port: (server.address() as AddressInfo).port,
    messages,
    close: () =>
      new Promise<void>((r) => {
        sockets.forEach((s) => s.destroy());
        server.close(() => r());
      }),
  };
}
