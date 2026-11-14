/**
 * Create a share link for a file on a cloud connection. Only options the
 * provider supports and the sharing policy allows are offered; the
 * backend checks the policy again before asking the provider.
 */
import { useEffect, useMemo, useState } from "react";
import { Copy, Link2, X } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { errorText } from "@/lib/error-text";
import { PROVIDER_CAPS, allowedExpiries, sharing, type ShareLink, type ShareScope, type SharingPolicy } from "@/stores/share-store";

const field = "w-full rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";

export function ShareLinkDialog({
  protocol,
  connectionId,
  path,
  name,
  onClose,
}: {
  protocol: string;
  connectionId: string | null;
  path: string;
  name: string;
  onClose: () => void;
}) {
  const caps = PROVIDER_CAPS[protocol] ?? { expiry: "optional", password: false, organization: false, edit: false };
  const [policy, setPolicy] = useState<SharingPolicy | null>(null);
  const [scope, setScope] = useState<ShareScope>("anyone");
  const [expiry, setExpiry] = useState<string>("");
  const [password, setPassword] = useState("");
  const [allowEdit, setAllowEdit] = useState(false);
  const [busy, setBusy] = useState(false);
  const [link, setLink] = useState<ShareLink | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const containerRef = useFocusTrap<HTMLDivElement>({ active: true, onEscape: onClose });

  useEffect(() => {
    sharing.policy().then(setPolicy, (e) => setError(errorText(e)));
  }, []);
  const expiries = useMemo(() => allowedExpiries(protocol, policy), [protocol, policy]);
  useEffect(() => {
    if (expiries.length && !expiries.some((e) => String(e.secs) === expiry)) setExpiry(String(expiries.find((e) => e.secs === 86_400)?.secs ?? expiries[0].secs));
  }, [expiries, expiry]);
  useEffect(() => {
    if (policy && !policy.allow_anyone && caps.organization) setScope("organization");
  }, [policy, caps.organization]);

  const blocked = policy && !policy.enabled ? "Creating share links is turned off by the sharing policy." : policy && !policy.allow_anyone && !caps.organization ? "The sharing policy only allows organization links, which this provider can't make." : expiries.length === 0 ? "No expiry allowed by both this provider and the sharing policy." : null;
  const needsPassword = !!policy?.require_password && scope === "anyone";

  const create = async () => {
    setBusy(true);
    setError(null);
    try {
      setLink(
        await sharing.create(protocol, connectionId, path, {
          expires_in_secs: expiry === "null" ? null : Number(expiry),
          password: password || null,
          scope,
          allow_edit: allowEdit,
        }),
      );
      setPassword("");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div ref={containerRef} className="fixed inset-0 z-[60] flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-labelledby="share-title">
      <div className="absolute inset-0 bg-black/50" aria-hidden="true" onClick={onClose} />
      <div className="relative z-10 w-full max-w-md rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl text-xs">
        <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 id="share-title" className="text-sm font-semibold truncate">Share “{name}”</h2>
          <Button variant="ghost" size="icon" onClick={onClose} aria-label="Close">
            <X className="h-4 w-4" />
          </Button>
        </div>
        <div className="space-y-3 px-4 py-3">
          {blocked && <p className="text-amber-600" role="alert">{blocked}</p>}
          {!link && !blocked && (
            <>
              {caps.organization && (
                <label className="block">
                  <span className="font-medium">Who can open it</span>
                  <select className={field} value={scope} onChange={(e) => setScope(e.target.value as ShareScope)} aria-label="Who can open it">
                    {(policy?.allow_anyone ?? true) && <option value="anyone">Anyone with the link</option>}
                    <option value="organization">People in my organization</option>
                  </select>
                </label>
              )}
              <label className="block">
                <span className="font-medium">Expires after</span>
                <select className={field} value={expiry} onChange={(e) => setExpiry(e.target.value)} aria-label="Expires after">
                  {expiries.map((e) => (
                    <option key={e.label} value={String(e.secs)}>{e.label}</option>
                  ))}
                </select>
              </label>
              {caps.password && (
                <label className="block">
                  <span className="font-medium">Password{needsPassword ? " (required by policy)" : " (optional)"}</span>
                  <input type="password" autoComplete="new-password" className={field} value={password} onChange={(e) => setPassword(e.target.value)} aria-label="Link password" />
                </label>
              )}
              {caps.edit && (policy?.allow_edit ?? true) && (
                <label className="flex items-center gap-1">
                  <input type="checkbox" checked={allowEdit} onChange={(e) => setAllowEdit(e.target.checked)} /> Allow editing
                </label>
              )}
              {needsPassword && !caps.password && <p className="text-amber-600">The policy requires a password, which this provider can't set.</p>}
            </>
          )}
          {link && (
            <div className="space-y-1.5" aria-label="Share link">
              <div className="flex gap-1">
                <input readOnly className={`${field} font-mono`} value={link.url} aria-label="Link" onFocus={(e) => e.currentTarget.select()} />
                <Button
                  size="sm"
                  variant="outline"
                  onClick={async () => {
                    await navigator.clipboard.writeText(link.url).catch(() => {});
                    setCopied(true);
                  }}
                >
                  <Copy className="h-3.5 w-3.5 mr-1" />
                  {copied ? "Copied" : "Copy"}
                </Button>
              </div>
              <p className="text-[color:var(--color-text-secondary)]">
                {link.scope === "anyone" ? "Anyone with the link" : "People in your organization"} ·{" "}
                {link.expires_at ? `expires ${new Date(link.expires_at).toLocaleString()}` : "does not expire"}
              </p>
              {link.note && <p className="text-[color:var(--color-text-tertiary)]">{link.note}</p>}
            </div>
          )}
          {error && <p className="text-red-500" role="alert">{error}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-[var(--color-border)] px-4 py-3">
          <Button variant="ghost" size="sm" onClick={onClose}>{link ? "Done" : "Cancel"}</Button>
          {!link && (
            <Button size="sm" disabled={!!blocked || busy || (needsPassword && (!caps.password || !password))} onClick={create}>
              <Link2 className="h-3.5 w-3.5 mr-1" /> {busy ? "Creating…" : "Create link"}
            </Button>
          )}
        </div>
      </div>
    </div>
  );
}
