/** A bounded TCP reachability probe. This never claims remote authentication succeeded. */
import net from "node:net";
import { guardedLookup, literalPrivate } from "./delivery";
import type { ConnectorDefinition } from "./db/repos/provisioning";
export async function probeConnector(
  c: ConnectorDefinition,
): Promise<{ reachable: boolean; error: string | null; durationMs: number }> {
  const host =
    c.protocol === "s3" && !c.host.includes(".")
      ? `s3.${c.host}.amazonaws.com`
      : c.host;
  const started = Date.now(),
    allow = process.env.CONNECTORS_ALLOW_PRIVATE === "1";
  if (literalPrivate(host, allow))
    return {
      reachable: false,
      error:
        "Private endpoints require CONNECTORS_ALLOW_PRIVATE=1 on the console",
      durationMs: 0,
    };
  const port =
    c.port ??
    {
      sftp: 22,
      ftp: 21,
      ftps: 990,
      webdav: 443,
      smb: 445,
      nfs: 2049,
      s3: 443,
      azure_blob: 443,
      adls_gen2: 443,
      azure_files: 443,
      b2: 443,
    }[c.protocol];
  return new Promise((resolve) => {
    const socket = net.createConnection({
      host: host.replace(/^\[|\]$/g, ""),
      port,
      lookup: guardedLookup(
        allow,
        "CONNECTORS_ALLOW_PRIVATE",
      ) as net.LookupFunction,
    });
    let finished = false;
    const done = (error: string | null) => {
      if (finished) return;
      finished = true;
      clearTimeout(deadline);
      socket.destroy();
      resolve({
        reachable: error === null,
        error,
        durationMs: Date.now() - started,
      });
    };
    const deadline = setTimeout(
      () => done("No response within 5 seconds"),
      5000,
    );
    socket.once("connect", () => done(null));
    socket.once("error", (e) => done(e.message));
  });
}
