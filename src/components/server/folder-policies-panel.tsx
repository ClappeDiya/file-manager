/**
 * Folder policies of folders ufop-server shares (`/api/v1/folder-policies`):
 * lock, accepted file types, upload size, link rules and intake naming.
 * Unset options inherit from the parent folder; the panel shows what is in
 * force and where it comes from, and can copy a policy to another folder.
 */
import { useState } from "react";
import { errorText } from "@/lib/error-text";
import { serverApi } from "@/lib/server-client";

export type IntakeNaming = "keep" | "timestamp" | "sequence";

export interface FolderPolicy {
  locked: boolean | null;
  allowed_extensions: string[] | null;
  blocked_extensions: string[] | null;
  max_upload_mb: number | null;
  require_link_password: boolean | null;
  max_link_days: number | null;
  intake_naming: IntakeNaming | null;
}

interface Effective {
  locked: boolean;
  allowed_extensions: string[] | null;
  blocked_extensions: string[];
  max_upload_mb: number | null;
  require_link_password: boolean;
  max_link_days: number | null;
  intake_naming: IntakeNaming;
  sources: string[];
}

interface PolicyView {
  path: string;
  own: FolderPolicy | null;
  effective: Effective;
}

const EMPTY: FolderPolicy = {
  locked: null,
  allowed_extensions: null,
  blocked_extensions: null,
  max_upload_mb: null,
  require_link_password: null,
  max_link_days: null,
  intake_naming: null,
};
const input =
  "w-full px-2 py-1 text-sm rounded border border-zinc-300 dark:border-zinc-600 bg-white dark:bg-zinc-800";
const tri = (v: boolean | null) => (v === null ? "" : v ? "yes" : "no");
const fromTri = (v: string) => (v === "" ? null : v === "yes");
const exts = (v: string) => {
  const list = v
    .split(/[\s,]+/)
    .map((e) => e.trim())
    .filter(Boolean);
  return list.length ? list : null;
};
const num = (v: string) => (v.trim() === "" ? null : Number(v));

export function FolderPoliciesPanel({
  pickFolder,
}: {
  pickFolder?: (title: string) => Promise<string | null>;
}) {
  const [path, setPath] = useState("");
  const [view, setView] = useState<PolicyView | null>(null);
  const [draft, setDraft] = useState<FolderPolicy>(EMPTY);
  const [copyTo, setCopyTo] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);

  const show = (v: PolicyView) => {
    setView(v);
    setDraft(v.own ?? EMPTY);
  };
  const run = async (f: () => Promise<void>) => {
    setError(null);
    setNote(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const load = (p = path) =>
    run(async () =>
      show(
        await serverApi<PolicyView>(
          "GET",
          `/folder-policies/effective?path=${encodeURIComponent(p)}`,
        ),
      ),
    );
  const save = (policy: FolderPolicy) =>
    run(async () => {
      show(
        await serverApi<PolicyView>("PUT", "/folder-policies", {
          path: view?.path ?? path,
          policy,
        }),
      );
      setNote("Saved.");
    });
  const copy = () =>
    run(async () => {
      await serverApi("POST", "/folder-policies/copy", {
        from: view?.path ?? path,
        to: copyTo,
      });
      setNote(`Copied to ${copyTo}.`);
    });

  const e = view?.effective;
  return (
    <section
      className="space-y-2 rounded-lg border border-zinc-200 dark:border-zinc-700 p-3"
      data-testid="folder-policies"
    >
      <h2 className="font-semibold">Folder policies</h2>
      <p className="text-xs text-zinc-500">
        Rules for a shared folder and its subfolders. Empty fields inherit from
        the parent folder.
      </p>
      <div className="flex gap-1">
        <input
          aria-label="Folder"
          className={input}
          value={path}
          onChange={(ev) => setPath(ev.target.value)}
          placeholder="/srv/share/inbox"
          onKeyDown={(ev) => ev.key === "Enter" && void load()}
        />
        {pickFolder && (
          <button
            type="button"
            className="px-2 rounded bg-zinc-100 dark:bg-zinc-800 text-xs"
            onClick={async () => {
              const p = await pickFolder("Folder");
              if (p) {
                setPath(p);
                void load(p);
              }
            }}
          >
            Browse…
          </button>
        )}
        <button
          type="button"
          className="px-3 rounded bg-blue-500 text-white text-xs disabled:opacity-50"
          disabled={!path.trim()}
          onClick={() => void load()}
        >
          Open
        </button>
      </div>
      {view && e && (
        <div className="space-y-2 text-xs">
          <p className="font-mono truncate" title={view.path}>
            {view.path}
          </p>
          <div className="grid gap-2 sm:grid-cols-2">
            <label>
              Locked (no uploads)
              <select
                aria-label="Locked"
                className={input}
                value={tri(draft.locked)}
                onChange={(ev) =>
                  setDraft({ ...draft, locked: fromTri(ev.target.value) })
                }
              >
                <option value="">
                  Inherit ({e.locked ? "locked" : "open"})
                </option>
                <option value="yes">Locked</option>
                <option value="no">Open</option>
              </select>
            </label>
            <label>
              Upload naming (intake order)
              <select
                aria-label="Intake naming"
                className={input}
                value={draft.intake_naming ?? ""}
                onChange={(ev) =>
                  setDraft({
                    ...draft,
                    intake_naming: (ev.target.value ||
                      null) as IntakeNaming | null,
                  })
                }
              >
                <option value="">Inherit ({e.intake_naming})</option>
                <option value="keep">Keep names</option>
                <option value="timestamp">Arrival time prefix</option>
                <option value="sequence">Sequence number prefix</option>
              </select>
            </label>
            <label>
              Accepted extensions
              <input
                aria-label="Accepted extensions"
                className={input}
                value={draft.allowed_extensions?.join(", ") ?? ""}
                placeholder={
                  e.allowed_extensions
                    ? `Inherit: ${e.allowed_extensions.join(", ")}`
                    : "Any"
                }
                onChange={(ev) =>
                  setDraft({
                    ...draft,
                    allowed_extensions: exts(ev.target.value),
                  })
                }
              />
            </label>
            <label>
              Blocked extensions
              <input
                aria-label="Blocked extensions"
                className={input}
                value={draft.blocked_extensions?.join(", ") ?? ""}
                placeholder={
                  e.blocked_extensions.length
                    ? `Inherit: ${e.blocked_extensions.join(", ")}`
                    : "None"
                }
                onChange={(ev) =>
                  setDraft({
                    ...draft,
                    blocked_extensions: exts(ev.target.value),
                  })
                }
              />
            </label>
            <label>
              Largest upload (MB)
              <input
                aria-label="Largest upload MB"
                type="number"
                min={1}
                className={input}
                value={draft.max_upload_mb ?? ""}
                placeholder={
                  e.max_upload_mb
                    ? `Inherit: ${e.max_upload_mb}`
                    : "Link's limit"
                }
                onChange={(ev) =>
                  setDraft({ ...draft, max_upload_mb: num(ev.target.value) })
                }
              />
            </label>
            <label>
              Links expire within (days)
              <input
                aria-label="Links expire within days"
                type="number"
                min={1}
                className={input}
                value={draft.max_link_days ?? ""}
                placeholder={
                  e.max_link_days ? `Inherit: ${e.max_link_days}` : "No limit"
                }
                onChange={(ev) =>
                  setDraft({ ...draft, max_link_days: num(ev.target.value) })
                }
              />
            </label>
            <label>
              Links need a password
              <select
                aria-label="Links need a password"
                className={input}
                value={tri(draft.require_link_password)}
                onChange={(ev) =>
                  setDraft({
                    ...draft,
                    require_link_password: fromTri(ev.target.value),
                  })
                }
              >
                <option value="">
                  Inherit ({e.require_link_password ? "yes" : "no"})
                </option>
                <option value="yes">Yes</option>
                <option value="no">No</option>
              </select>
            </label>
          </div>
          <p className="text-zinc-500">
            In force: {e.locked ? "locked" : "open"} ·{" "}
            {e.allowed_extensions
              ? `only ${e.allowed_extensions.join(", ")}`
              : "any type"}
            {e.blocked_extensions.length > 0 &&
              ` except ${e.blocked_extensions.join(", ")}`}{" "}
            · naming {e.intake_naming}
            {e.sources.length > 0 &&
              ` · from ${e.sources.length} folder${e.sources.length === 1 ? "" : "s"}`}
          </p>
          <div className="flex flex-wrap gap-2 items-center">
            <button
              type="button"
              className="px-3 py-1 rounded bg-blue-500 text-white"
              onClick={() => void save(draft)}
            >
              Save
            </button>
            {view.own && (
              <button
                type="button"
                className="px-3 py-1 rounded border border-zinc-300 dark:border-zinc-600"
                onClick={() => void save(EMPTY)}
              >
                Remove this folder&apos;s policy
              </button>
            )}
            {view.own && (
              <>
                <input
                  aria-label="Copy to folder"
                  className={`${input} max-w-xs`}
                  value={copyTo}
                  onChange={(ev) => setCopyTo(ev.target.value)}
                  placeholder="Copy to folder…"
                />
                <button
                  type="button"
                  className="px-3 py-1 rounded border border-zinc-300 dark:border-zinc-600 disabled:opacity-50"
                  disabled={!copyTo.trim()}
                  onClick={() => void copy()}
                >
                  Copy
                </button>
              </>
            )}
          </div>
        </div>
      )}
      {note && (
        <p className="text-xs text-green-600" role="status">
          {note}
        </p>
      )}
      {error && (
        <p className="text-xs text-red-600" role="alert">
          {error}
        </p>
      )}
    </section>
  );
}
