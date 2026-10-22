"use client";

/** Asks the user to confirm who they are before a sensitive change (see `reauth-store`). */
import React, { useState } from "react";
import { Dialog } from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ErrorBanner } from "@/components/fleet/badges";
import { api, errorText } from "@/lib/api-client";
import { getPasskey, passkeysSupported } from "@/lib/passkey-client";
import { useReauthStore } from "@/lib/stores/reauth-store";

export function ReauthDialog() {
  const methods = useReauthStore((s) => s.methods);
  const finish = useReauthStore((s) => s.finish);
  const [value, setValue] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  if (!methods) return null;

  const done = (ok: boolean) => {
    setValue("");
    setErr(null);
    finish(ok);
  };
  const run = async (f: () => Promise<unknown>) => {
    setBusy(true);
    setErr(null);
    try {
      await f();
      done(true);
    } catch (e) {
      setErr(errorText(e));
    } finally {
      setBusy(false);
    }
  };
  // The re-auth endpoint never answers reauth_required, so these calls cannot loop.
  const withPasskey = () =>
    run(async () => {
      const o = await api<{
        challengeId: string;
        publicKey: Parameters<typeof getPasskey>[0];
      }>("/api/auth/reauth", { body: { method: "passkey_options" } });
      await api("/api/auth/reauth", {
        body: {
          method: "passkey",
          challengeId: o.challengeId,
          credential: await getPasskey(o.publicKey),
        },
      });
    });
  const typed = methods.includes("totp")
    ? "totp"
    : methods.includes("password")
      ? "password"
      : null;

  return (
    <Dialog open onClose={() => done(false)} className="max-w-md">
      <h2 className="text-lg font-semibold">Confirm it is you</h2>
      <p className="mt-1 text-sm text-muted-foreground">
        This change needs a recent sign-in.
      </p>
      <ErrorBanner message={err} />
      <form
        className="mt-4 space-y-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (typed)
            void run(() =>
              api("/api/auth/reauth", {
                body:
                  typed === "totp"
                    ? { method: "totp", code: value }
                    : { method: "password", password: value },
              }),
            );
        }}
      >
        {typed && (
          <Input
            label={
              typed === "totp"
                ? "Code from your authenticator app (or a recovery code)"
                : "Your password"
            }
            type={typed === "totp" ? "text" : "password"}
            autoComplete={
              typed === "totp" ? "one-time-code" : "current-password"
            }
            value={value}
            onChange={(e) => setValue(e.target.value)}
            autoFocus
          />
        )}
        {methods.includes("sso") && (
          <p className="text-sm">
            This account signs in with single sign-on. Sign in again, then
            repeat the change.
          </p>
        )}
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="secondary" onClick={() => done(false)}>
            Cancel
          </Button>
          {methods.includes("passkey") && passkeysSupported() && (
            <Button
              type="button"
              variant={typed ? "secondary" : "primary"}
              disabled={busy}
              onClick={() => void withPasskey()}
            >
              Use a passkey
            </Button>
          )}
          {methods.includes("sso") && (
            <Button
              type="button"
              onClick={() =>
                window.location.assign(
                  `/login?redirect=${encodeURIComponent(window.location.pathname)}`,
                )
              }
            >
              Sign in again
            </Button>
          )}
          {typed && (
            <Button type="submit" disabled={busy || !value}>
              Confirm
            </Button>
          )}
        </div>
      </form>
    </Dialog>
  );
}
