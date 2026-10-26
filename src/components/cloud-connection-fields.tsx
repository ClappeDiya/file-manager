/**
 * Where a Google Drive / OneDrive / Dropbox connection opens (business
 * scopes), saved on the profile's `extras`:
 * - Google Drive: My Drive, Shared with me or one shared drive (picker);
 *   Docs export formats (Advanced).
 * - OneDrive: personal or work account; a SharePoint site's document
 *   library (site search + library picker).
 * - Dropbox: the member's folder, the team space or a team folder.
 * Pickers need the connection to be signed in.
 */
import { useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { errorText } from "@/lib/error-text";
import {
  GOOGLE_EXPORT_CHOICES,
  listDropboxNamespaces,
  listSharedDrives,
  listSiteLibraries,
  searchSharePointSites,
  type DropboxNamespace,
  type SharePointLibrary,
  type SharePointSite,
  type SharedDrive,
} from "@/lib/oauth";

/** Mirrors `OAuthSettings` (core/types.rs). */
export interface OAuthSettings {
  client_id: string | null;
  account_kind: "personal" | "business";
}

/** Mirrors `GoogleExportFormats`. */
export interface GoogleExportFormats {
  document: string | null;
  spreadsheet: string | null;
  presentation: string | null;
  drawing: string | null;
}

/** Mirrors `GoogleDriveSettings`. */
export interface GoogleDriveSettings {
  root: "my_drive" | "shared_with_me" | "shared_drive";
  shared_drive_id: string | null;
  shared_drive_name: string | null;
  export_formats: GoogleExportFormats;
}

/** Mirrors `OneDriveSettings`. */
export interface OneDriveSettings {
  site_id: string | null;
  site_name: string | null;
  drive_id: string | null;
  drive_name: string | null;
}

/** Mirrors `DropboxSettings`. */
export interface DropboxSettings {
  root_namespace_id: string | null;
  root_label: string | null;
}

export interface CloudExtras {
  oauth?: OAuthSettings | null;
  google_drive?: GoogleDriveSettings | null;
  onedrive?: OneDriveSettings | null;
  dropbox?: DropboxSettings | null;
}

const NO_EXPORTS: GoogleExportFormats = { document: null, spreadsheet: null, presentation: null, drawing: null };
const input = "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";
const small = "text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800 disabled:opacity-50";
const hint = "text-[10px] text-gray-400 dark:text-gray-500";

function SignInFirst({ what }: { what: string }) {
  return <p className={hint}>Sign in to this connection first to choose {what}.</p>;
}

export function CloudScopeFields({
  protocol,
  connectionId,
  value,
  onChange,
}: {
  protocol: string;
  /** Saved and signed-in connection the pickers query; `null` = not yet. */
  connectionId: string | null;
  value: CloudExtras;
  onChange: (next: CloudExtras) => void;
}) {
  if (protocol === "google_drive") return <GoogleDriveFields connectionId={connectionId} value={value} onChange={onChange} />;
  if (protocol === "onedrive") return <OneDriveFields connectionId={connectionId} value={value} onChange={onChange} />;
  if (protocol === "dropbox") return <DropboxFields connectionId={connectionId} value={value} onChange={onChange} />;
  return null;
}

type FieldsProps = { connectionId: string | null; value: CloudExtras; onChange: (next: CloudExtras) => void };

function GoogleDriveFields({ connectionId, value, onChange }: FieldsProps) {
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const g: GoogleDriveSettings = value.google_drive ?? { root: "my_drive", shared_drive_id: null, shared_drive_name: null, export_formats: NO_EXPORTS };
  const set = (patch: Partial<GoogleDriveSettings>) => onChange({ ...value, google_drive: { ...g, ...patch } });
  const [drives, setDrives] = useState<SharedDrive[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = async () => {
    if (!connectionId) return;
    setError(null);
    try {
      setDrives(await listSharedDrives(connectionId));
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="space-y-2" data-testid="gdrive-scope">
      <div>
        <label className="block text-xs font-medium mb-1" htmlFor="gdrive-root">Open</label>
        <select id="gdrive-root" value={g.root} onChange={(e) => set({ root: e.target.value as GoogleDriveSettings["root"] })} className={input}>
          <option value="my_drive">My Drive</option>
          <option value="shared_with_me">Shared with me</option>
          <option value="shared_drive">A shared drive</option>
        </select>
      </div>
      {g.root === "shared_drive" && (
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <select
              aria-label="Shared drive"
              value={g.shared_drive_id ?? ""}
              onChange={(e) => {
                const d = drives?.find((x) => x.id === e.target.value);
                set({ shared_drive_id: e.target.value || null, shared_drive_name: d?.name ?? null });
              }}
              className={input}
            >
              <option value="">{g.shared_drive_name && !drives ? g.shared_drive_name : "Choose a shared drive…"}</option>
              {(drives ?? (g.shared_drive_id ? [{ id: g.shared_drive_id, name: g.shared_drive_name ?? g.shared_drive_id }] : [])).map((d) => (
                <option key={d.id} value={d.id}>{d.name}</option>
              ))}
            </select>
            <button type="button" className={small} disabled={!connectionId} onClick={() => void load()}>
              Load
            </button>
          </div>
          {!connectionId && <SignInFirst what="a shared drive" />}
          {drives?.length === 0 && <p className={hint}>This account has no shared drives.</p>}
        </div>
      )}
      {advanced && (
        <details className="rounded border border-gray-200 dark:border-gray-700 p-2">
          <summary className="text-xs font-medium cursor-pointer">Google Docs export formats</summary>
          <div className="grid grid-cols-2 gap-2 mt-2">
            {(Object.keys(GOOGLE_EXPORT_CHOICES) as (keyof typeof GOOGLE_EXPORT_CHOICES)[]).map((kind) => (
              <div key={kind}>
                <label className="block text-xs font-medium mb-1 capitalize" htmlFor={`gdrive-export-${kind}`}>{kind === "document" ? "Docs" : kind === "spreadsheet" ? "Sheets" : kind === "presentation" ? "Slides" : "Drawings"}</label>
                <select
                  id={`gdrive-export-${kind}`}
                  value={g.export_formats[kind] ?? ""}
                  onChange={(e) => set({ export_formats: { ...g.export_formats, [kind]: e.target.value || null } })}
                  className={input}
                >
                  <option value="">Default ({GOOGLE_EXPORT_CHOICES[kind][0]})</option>
                  {GOOGLE_EXPORT_CHOICES[kind].map((f) => (
                    <option key={f} value={f}>{f}</option>
                  ))}
                </select>
              </div>
            ))}
          </div>
          <p className={`${hint} mt-1`}>Native Google files are exported in these formats when downloaded or copied.</p>
        </details>
      )}
      {error && <p className="text-xs text-red-600 dark:text-red-400" role="alert">{error}</p>}
    </div>
  );
}

function OneDriveFields({ connectionId, value, onChange }: FieldsProps) {
  const oauth: OAuthSettings = value.oauth ?? { client_id: null, account_kind: "personal" };
  const lib: OneDriveSettings = value.onedrive ?? { site_id: null, site_name: null, drive_id: null, drive_name: null };
  const [query, setQuery] = useState("");
  const [sites, setSites] = useState<SharePointSite[] | null>(null);
  const [libraries, setLibraries] = useState<SharePointLibrary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const run = async <T,>(f: () => Promise<T>, then: (v: T) => void) => {
    setError(null);
    try {
      then(await f());
    } catch (e) {
      setError(errorText(e));
    }
  };
  return (
    <div className="space-y-2" data-testid="onedrive-scope">
      <div>
        <label className="block text-xs font-medium mb-1" htmlFor="onedrive-account">Account</label>
        <select
          id="onedrive-account"
          value={oauth.account_kind}
          onChange={(e) => onChange({ ...value, oauth: { ...oauth, account_kind: e.target.value as OAuthSettings["account_kind"] } })}
          className={input}
        >
          <option value="personal">Personal Microsoft account</option>
          <option value="business">Work or school (Microsoft 365)</option>
        </select>
      </div>
      {oauth.account_kind === "business" && (
        <div className="space-y-1">
          <label className="block text-xs font-medium" htmlFor="sharepoint-search">SharePoint library</label>
          <p className="text-xs" data-testid="onedrive-library">
            {lib.drive_id ? `${lib.site_name ?? "Site"} › ${lib.drive_name ?? lib.drive_id}` : "My OneDrive"}
            {lib.drive_id && (
              <button type="button" className="ml-2 text-blue-600 dark:text-blue-400 hover:underline" onClick={() => onChange({ ...value, onedrive: null })}>
                Use my OneDrive
              </button>
            )}
          </p>
          <div className="flex gap-2">
            <input id="sharepoint-search" value={query} onChange={(e) => setQuery(e.target.value)} placeholder="Search sites" className={input} />
            <button type="button" className={small} disabled={!connectionId} onClick={() => void run(() => searchSharePointSites(connectionId!, query), setSites)}>
              Search
            </button>
          </div>
          {!connectionId && <SignInFirst what="a SharePoint library" />}
          {sites && (
            <select
              aria-label="SharePoint site"
              value={lib.site_id ?? ""}
              onChange={(e) => {
                const s = sites.find((x) => x.id === e.target.value);
                onChange({ ...value, onedrive: { site_id: s?.id ?? null, site_name: s?.display_name ?? null, drive_id: null, drive_name: null } });
                setLibraries(null);
                if (s && connectionId) void run(() => listSiteLibraries(connectionId, s.id), setLibraries);
              }}
              className={input}
            >
              <option value="">{sites.length ? "Choose a site…" : "No sites found"}</option>
              {sites.map((s) => (
                <option key={s.id} value={s.id}>{s.display_name}</option>
              ))}
            </select>
          )}
          {libraries && (
            <select
              aria-label="Document library"
              value={lib.drive_id ?? ""}
              onChange={(e) => {
                const l = libraries.find((x) => x.id === e.target.value);
                onChange({ ...value, onedrive: { ...lib, drive_id: l?.id ?? null, drive_name: l?.name ?? null } });
              }}
              className={input}
            >
              <option value="">Choose a library…</option>
              {libraries.map((l) => (
                <option key={l.id} value={l.id}>{l.name}</option>
              ))}
            </select>
          )}
        </div>
      )}
      {error && <p className="text-xs text-red-600 dark:text-red-400" role="alert">{error}</p>}
    </div>
  );
}

function DropboxFields({ connectionId, value, onChange }: FieldsProps) {
  const d: DropboxSettings = value.dropbox ?? { root_namespace_id: null, root_label: null };
  const [spaces, setSpaces] = useState<DropboxNamespace[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const load = async () => {
    if (!connectionId) return;
    setError(null);
    try {
      setSpaces(await listDropboxNamespaces(connectionId));
    } catch (e) {
      setError(errorText(e));
    }
  };
  const options = spaces ?? [{ namespace_id: d.root_namespace_id, label: d.root_label ?? (d.root_namespace_id ? d.root_namespace_id : "My Dropbox"), kind: "saved" }];
  return (
    <div className="space-y-1" data-testid="dropbox-scope">
      <label className="block text-xs font-medium mb-1" htmlFor="dropbox-root">Open</label>
      <div className="flex gap-2">
        <select
          id="dropbox-root"
          value={d.root_namespace_id ?? ""}
          onChange={(e) => {
            const ns = options.find((o) => (o.namespace_id ?? "") === e.target.value);
            onChange({ ...value, dropbox: e.target.value ? { root_namespace_id: e.target.value, root_label: ns?.label ?? null } : null });
          }}
          className={input}
        >
          {!options.some((o) => !o.namespace_id) && <option value="">My Dropbox</option>}
          {options.map((o) => (
            <option key={o.namespace_id ?? "home"} value={o.namespace_id ?? ""}>
              {o.label}
              {o.kind === "team_folder" ? " (team folder)" : ""}
            </option>
          ))}
        </select>
        <button type="button" className={small} disabled={!connectionId} onClick={() => void load()}>
          Load team spaces
        </button>
      </div>
      {!connectionId && <SignInFirst what="a team space or team folder" />}
      {error && <p className="text-xs text-red-600 dark:text-red-400" role="alert">{error}</p>}
    </div>
  );
}
