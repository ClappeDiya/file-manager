/**
 * Web UI served by `ufop-server`: sign-in screen, then the Backups panel
 * and Share links. Folder pickers browse the server's filesystem.
 */
import { useEffect, useState } from "react";
import { BackupPanel } from "@/components/backup/backup-panel";
import { errorText } from "@/lib/error-text";
import { pickFolder, setFolderPickerImpl } from "@/lib/folder-picker";
import { ShareLinksPanel } from "@/components/server/share-links-panel";
import { GatewaysPanel } from "@/components/server/gateways-panel";
import {
  isSignedIn,
  login,
  logout,
  onAuthChange,
  refresh,
  serverRpc,
} from "@/lib/server-client";

interface DirListing {
  path: string;
  parent: string | null;
  entries: { name: string; path: string; is_dir: boolean }[];
}

function FolderBrowser({
  title,
  onDone,
}: {
  title: string;
  onDone: (p: string | null) => void;
}) {
  const [listing, setListing] = useState<DirListing | null>(null);
  const [error, setError] = useState<string | null>(null);
  const open = (path: string | null) =>
    serverRpc<DirListing>("server_list_directory", { path })
      .then((l) => {
        setListing(l);
        setError(null);
      })
      .catch((e) => setError(errorText(e)));
  useEffect(() => {
    void open(null);
  }, []);
  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50"
      role="dialog"
      aria-modal="true"
      aria-label={title}
    >
      <div className="w-[min(560px,94vw)] max-h-[80vh] flex flex-col rounded-lg bg-white dark:bg-zinc-900 border border-zinc-200 dark:border-zinc-700">
        <div className="p-3 border-b border-zinc-200 dark:border-zinc-700 text-sm font-semibold">
          {title}
        </div>
        <div className="px-3 py-2 text-xs font-mono truncate text-zinc-600 dark:text-zinc-400">
          {listing?.path ?? "…"}
        </div>
        {error && <div className="px-3 text-xs text-red-600">{error}</div>}
        <ul
          className="flex-1 overflow-y-auto text-sm"
          data-testid="server-folder-list"
        >
          {listing?.parent && (
            <li>
              <button
                className="w-full text-left px-3 py-1 hover:bg-zinc-100 dark:hover:bg-zinc-800"
                onClick={() => void open(listing.parent)}
              >
                ⬆ ..
              </button>
            </li>
          )}
          {listing?.entries.map((e) => (
            <li key={e.path}>
              <button
                className="w-full text-left px-3 py-1 hover:bg-zinc-100 dark:hover:bg-zinc-800 truncate"
                onClick={() => void open(e.path)}
              >
                📁 {e.name}
              </button>
            </li>
          ))}
        </ul>
        <div className="flex justify-end gap-2 p-3 border-t border-zinc-200 dark:border-zinc-700">
          <button
            className="px-3 py-1 text-xs rounded bg-zinc-100 dark:bg-zinc-800"
            onClick={() => onDone(null)}
          >
            Cancel
          </button>
          <button
            className="px-3 py-1 text-xs rounded bg-blue-500 text-white"
            disabled={!listing}
            onClick={() => onDone(listing?.path ?? null)}
          >
            Select this folder
          </button>
        </div>
      </div>
    </div>
  );
}

function SignIn() {
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  return (
    <div className="min-h-full flex items-center justify-center p-4 bg-zinc-50 dark:bg-zinc-950">
      <form
        className="w-full max-w-sm space-y-3 rounded-lg border border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900 p-6"
        onSubmit={async (e) => {
          e.preventDefault();
          setBusy(true);
          setError(null);
          try {
            await login(password);
          } catch (err) {
            setError(errorText(err));
          } finally {
            setBusy(false);
          }
        }}
      >
        <h1 className="text-lg font-semibold text-zinc-900 dark:text-zinc-100">
          UFOP backup server
        </h1>
        <label className="block text-sm text-zinc-700 dark:text-zinc-300">
          Password
          <input
            id="server-password"
            type="password"
            autoFocus
            autoComplete="current-password"
            className="mt-1 w-full px-2 py-1.5 rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800"
            value={password}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        {error && (
          <p className="text-sm text-red-600" role="alert">
            {error}
          </p>
        )}
        <button
          type="submit"
          disabled={busy || !password}
          className="w-full py-1.5 rounded bg-blue-500 text-white text-sm disabled:opacity-50"
        >
          {busy ? "Signing in…" : "Sign in"}
        </button>
      </form>
    </div>
  );
}

export default function ServerApp() {
  const [signedIn, setSignedIn] = useState(isSignedIn());
  const [checking, setChecking] = useState(true);
  const [picker, setPicker] = useState<{
    title: string;
    resolve: (p: string | null) => void;
  } | null>(null);
  const [tab, setTab] = useState<"backups" | "shares" | "gateways">("backups");

  useEffect(() => {
    const off = onAuthChange(setSignedIn);
    // An existing refresh cookie signs us back in after a reload.
    void refresh().finally(() => setChecking(false));
    setFolderPickerImpl(
      (title) => new Promise((resolve) => setPicker({ title, resolve })),
    );
    return () => {
      off();
      setFolderPickerImpl(null);
    };
  }, []);

  if (checking)
    return <div className="p-6 text-sm text-zinc-500">Connecting…</div>;
  if (!signedIn) return <SignIn />;
  return (
    <div className="h-full flex flex-col">
      <div className="flex items-center gap-1 px-3 py-1 text-xs border-b border-zinc-200 dark:border-zinc-700 bg-white dark:bg-zinc-900">
        <div role="tablist" className="flex gap-1">
          {(
            [
              ["backups", "Backups"],
              ["shares", "Share links"],
              ["gateways", "Gateways"],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              role="tab"
              aria-selected={tab === id}
              className={`px-2 py-0.5 rounded ${tab === id ? "bg-blue-500 text-white" : "text-zinc-600 dark:text-zinc-400 hover:bg-zinc-100 dark:hover:bg-zinc-800"}`}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </div>
        <button
          className="ml-auto underline text-zinc-600 dark:text-zinc-400"
          onClick={() => void logout()}
        >
          Sign out
        </button>
      </div>
      <div className="flex-1 min-h-0">
        {tab === "backups" ? (
          <BackupPanel />
        ) : tab === "shares" ? (
          <ShareLinksPanel pickFolder={pickFolder} />
        ) : (
          <GatewaysPanel />
        )}
      </div>
      {picker && (
        <FolderBrowser
          title={picker.title}
          onDone={(p) => {
            picker.resolve(p);
            setPicker(null);
          }}
        />
      )}
    </div>
  );
}
