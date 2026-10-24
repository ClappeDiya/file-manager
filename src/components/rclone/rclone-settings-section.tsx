/**
 * Settings → rclone: which rclone binary and rclone.conf every rclone
 * feature uses (connections, backups, mounts, the remote manager), with
 * Detect, the version check and the minimum-version warning. Advanced:
 * install or update the app's own (managed) copy of rclone, verified
 * against the release's SHA256SUMS.
 */
import { useCallback, useEffect, useState } from "react";
import { AdvancedOnly } from "@/components/backup/advanced-only";
import { isTauriAvailable } from "@/hooks/use-tauri";
import { pickFiles } from "@/lib/folder-picker";
import { errorText } from "@/lib/error-text";
import {
  SOURCE_LABEL,
  describeStatus,
  detectRclone,
  getRcloneSettings,
  installManagedRclone,
  setRcloneSettings,
  type RcloneCandidate,
  type RcloneSettingsView,
  type RcloneStatus,
} from "@/lib/rclone";

const input = "flex-1 min-w-0 px-2 py-1 text-xs rounded border border-gray-200 dark:border-gray-700 bg-transparent font-mono";
const btn = "text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 disabled:opacity-50";

export function RcloneSettingsSection() {
  const [view, setView] = useState<RcloneSettingsView | null>(null);
  const [binary, setBinary] = useState("");
  const [config, setConfig] = useState("");
  const [candidates, setCandidates] = useState<RcloneCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [busy, setBusy] = useState<"save" | "detect" | "install" | null>(null);

  const show = useCallback((v: RcloneSettingsView) => {
    setView(v);
    setBinary(v.settings.binary_path ?? "");
    setConfig(v.settings.config_path ?? "");
  }, []);

  useEffect(() => {
    if (!isTauriAvailable()) return;
    getRcloneSettings().then(show, (e) => setError(errorText(e)));
  }, [show]);

  const save = async (b = binary, c = config) => {
    setBusy("save");
    setError(null);
    setNotice(null);
    try {
      show(await setRcloneSettings({ binary_path: b.trim() || null, config_path: c.trim() || null }));
      setNotice("Saved.");
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(null);
    }
  };

  if (!isTauriAvailable()) return <p className="pt-3 text-xs text-gray-400">Available in the desktop app.</p>;
  if (!view) return <p className="pt-3 text-xs text-gray-400">{error ?? "Loading…"}</p>;
  const s = view.status;

  return (
    <div className="pt-3 space-y-3 text-xs" data-testid="rclone-settings">
      <div className="space-y-0.5" data-testid="rclone-status">
        {s.version ? (
          <p>
            Using rclone <span className="font-mono">{s.version}</span> at <span className="font-mono break-all">{s.path}</span> ({s.managed ? "managed copy" : SOURCE_LABEL[s.source]})
          </p>
        ) : (
          <p className="text-red-600 dark:text-red-400">
            rclone is not installed{s.source !== "none" ? ` or does not run (${s.path}: ${s.error ?? "no version"})` : ""}. Install it from rclone.org, choose its path below
            {view.platform_supported ? ", or install the managed copy (Advanced)" : ""}.
          </p>
        )}
        {s.meets_minimum === false && (
          <p className="text-amber-600 dark:text-amber-400" role="alert" data-testid="rclone-min-warning">
            This rclone is older than {s.min_version}; creating remotes in the app and some features need {s.min_version} or newer. Update rclone.
          </p>
        )}
        <p className="text-gray-500">Config file: <span className="font-mono break-all">{s.config_path ?? view.default_config_path ?? "rclone's default"}</span></p>
      </div>

      <div className="space-y-1">
        <label className="block font-medium" htmlFor="rclone-binary">rclone program (blank = UFOP_RCLONE, then PATH)</label>
        <div className="flex gap-1">
          <input id="rclone-binary" value={binary} onChange={(e) => setBinary(e.target.value)} placeholder="/usr/local/bin/rclone" className={input} />
          <button
            type="button"
            className={btn}
            onClick={() =>
              void pickFiles("Choose the rclone program").then((f) => {
                if (f[0]) setBinary(f[0]);
              })
            }
          >
            Browse…
          </button>
          <button
            type="button"
            className={btn}
            disabled={busy !== null}
            onClick={() => {
              setBusy("detect");
              detectRclone().then(
                (c) => {
                  setCandidates(c);
                  setBusy(null);
                },
                (e) => {
                  setError(errorText(e));
                  setBusy(null);
                },
              );
            }}
          >
            {busy === "detect" ? "Detecting…" : "Detect"}
          </button>
        </div>
        {candidates && (
          <ul className="border border-gray-200 dark:border-gray-700 rounded divide-y divide-gray-100 dark:divide-gray-800" data-testid="rclone-candidates">
            {candidates.length === 0 && <li className="px-2 py-1 text-gray-500">No rclone found on this computer.</li>}
            {candidates.map((c) => (
              <li key={c.path} className="px-2 py-1 flex items-center gap-2">
                <span className="font-mono break-all">{c.path}</span>
                <span className="text-gray-500">{c.version ?? c.error}</span>
                {c.meets_minimum === false && <span className="text-amber-600">too old</span>}
                <button type="button" className={`${btn} ml-auto`} disabled={!c.version} onClick={() => void save(c.path, config)}>
                  Use
                </button>
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="space-y-1">
        <label className="block font-medium" htmlFor="rclone-config">rclone.conf (blank = rclone's default)</label>
        <div className="flex gap-1">
          <input id="rclone-config" value={config} onChange={(e) => setConfig(e.target.value)} placeholder={view.default_config_path ?? "~/.config/rclone/rclone.conf"} className={input} />
          <button
            type="button"
            className={btn}
            onClick={() =>
              void pickFiles("Choose rclone.conf").then((f) => {
                if (f[0]) setConfig(f[0]);
              })
            }
          >
            Browse…
          </button>
        </div>
      </div>

      <div className="flex items-center gap-2">
        <button type="button" className="text-xs px-3 py-1 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50" disabled={busy !== null} onClick={() => void save()}>
          {busy === "save" ? "Saving…" : "Save"}
        </button>
        {notice && <span className="text-green-600">{notice}</span>}
      </div>
      {error && (
        <p className="text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}

      <AdvancedOnly hint>
        <div className="space-y-1 border-t border-gray-200 dark:border-gray-700 pt-2" data-testid="rclone-managed">
          <h4 className="font-semibold">Managed rclone</h4>
          <p className="text-gray-500">
            Downloads rclone {view.pinned_version} from downloads.rclone.org, checks it against the release's SHA256SUMS and installs it into{" "}
            <span className="font-mono break-all">{view.managed_dir}</span>. A rclone you installed elsewhere is never replaced.
          </p>
          {view.managed_version && (
            <p>
              Installed: <span className="font-mono">{view.managed_version}</span>
              {view.update_available && <span className="text-amber-600"> — update to {view.pinned_version} available</span>}
            </p>
          )}
          <button
            type="button"
            className={btn}
            disabled={busy !== null || !view.platform_supported || (!!view.managed_version && !view.update_available)}
            onClick={() => {
              setBusy("install");
              setError(null);
              setNotice(null);
              installManagedRclone().then(
                (o) => {
                  show(o.view);
                  setNotice(`Installed rclone ${o.result.version} (SHA-256 verified).`);
                  setBusy(null);
                },
                (e) => {
                  setError(errorText(e));
                  setBusy(null);
                },
              );
            }}
          >
            {busy === "install" ? "Downloading…" : view.managed_version ? (view.update_available ? `Update to ${view.pinned_version}` : "Up to date") : `Install rclone ${view.pinned_version}`}
          </button>
          {!view.platform_supported && <p className="text-gray-500">No rclone build is published for this platform.</p>}
        </div>
      </AdvancedOnly>
    </div>
  );
}

/** About: the rclone engine in use, or "not installed". */
export function RcloneEngineInfo({ status }: { status: RcloneStatus | null }) {
  if (!status) return null;
  return (
    <div data-testid="about-rclone">
      rclone: {describeStatus(status)}
      {status.meets_minimum === false && ` — older than ${status.min_version}`}
    </div>
  );
}
