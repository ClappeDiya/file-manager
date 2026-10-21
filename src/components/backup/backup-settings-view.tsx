/**
 * Shared backup settings: global option defaults (applied to every option a
 * job leaves at the built-in value) and the destination library.
 */
import { useEffect, useMemo, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { diffOptions, inputCls } from "@/lib/backup-helpers";
import {
  errorText,
  newBackupJob,
  type BackupOptions,
  type GlobalDefaults,
  type SavedDestination,
} from "@/stores/backup-store";
import { BackupAdvancedOptions } from "./backup-advanced-options";
import { Banner, Btn, Field, PathInput, Section } from "./backup-ui";

const EMPTY: GlobalDefaults = { options: {}, throttle: null, retry: null };

function DefaultsEditor() {
  const builtin = useMemo(() => newBackupJob().options, []);
  const [opts, setOpts] = useState<BackupOptions>(builtin);
  const [uploadKbps, setUploadKbps] = useState(0);
  const [downloadKbps, setDownloadKbps] = useState(0);
  const [msg, setMsg] = useState<{ tone: "error" | "success"; text: string } | null>(null);

  useEffect(() => {
    tauriInvoke<GlobalDefaults>("backup_get_global_defaults", undefined, EMPTY)
      .then((d) => {
        setOpts({ ...builtin, ...d.options });
        setUploadKbps(Math.round((d.throttle?.upload_bps ?? 0) / 1024));
        setDownloadKbps(Math.round((d.throttle?.download_bps ?? 0) / 1024));
      })
      .catch((e) => setMsg({ tone: "error", text: errorText(e) }));
  }, [builtin]);

  const changed = diffOptions(builtin, opts);
  const save = async () => {
    const throttle = uploadKbps || downloadKbps ? { upload_bps: uploadKbps * 1024, download_bps: downloadKbps * 1024 } : null;
    try {
      await tauriInvoke("backup_set_global_defaults", { defaults: { options: changed, throttle, retry: null } satisfies GlobalDefaults });
      setMsg({ tone: "success", text: `Saved ${Object.keys(changed).length} default option(s).` });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    }
  };

  return (
    <div className="space-y-3" data-testid="backup-global-defaults">
      <Banner tone="info">
        Defaults apply to every job option still at its built-in value; a job&apos;s own setting always wins. {Object.keys(changed).length} option(s) currently differ from the built-in values.
      </Banner>
      {msg && <Banner tone={msg.tone} onClose={() => setMsg(null)}>{msg.text}</Banner>}
      <Section title="Basics">
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
          <Field label="Remote volume size (MB)">
            <input className={inputCls} type="number" min={1} value={Math.round(opts.volume_size / 1048576)} onChange={(e) => setOpts({ ...opts, volume_size: Math.max(1, Number(e.target.value)) * 1048576 })} />
          </Field>
          <Field label="Compression level (0 = store)">
            <input className={inputCls} type="number" min={0} max={9} value={opts.compression_level} onChange={(e) => setOpts({ ...opts, compression_level: Math.min(9, Math.max(0, Number(e.target.value))) })} />
          </Field>
          <Field label="Upload limit (KB/s, 0 = unlimited)">
            <input className={inputCls} type="number" min={0} value={uploadKbps} onChange={(e) => setUploadKbps(Math.max(0, Number(e.target.value)))} />
          </Field>
          <Field label="Download limit (KB/s, 0 = unlimited)">
            <input className={inputCls} type="number" min={0} value={downloadKbps} onChange={(e) => setDownloadKbps(Math.max(0, Number(e.target.value)))} />
          </Field>
        </div>
      </Section>
      <BackupAdvancedOptions options={opts} locked={false} onChange={(p) => setOpts({ ...opts, ...p })} />
      <div className="flex gap-1">
        <Btn tone="primary" testId="backup-defaults-save" onClick={() => void save()}>
          Save defaults
        </Btn>
        <Btn onClick={() => setOpts(builtin)}>Reset to built-in</Btn>
      </div>
    </div>
  );
}

function DestinationLibrary() {
  const [items, setItems] = useState<SavedDestination[]>([]);
  const [name, setName] = useState("");
  const [path, setPath] = useState("");
  const [error, setError] = useState<string | null>(null);

  const load = () =>
    tauriInvoke<SavedDestination[]>("backup_list_destinations", undefined, [])
      .then(setItems)
      .catch((e) => setError(errorText(e)));
  useEffect(() => {
    void load();
  }, []);

  const add = async () => {
    try {
      await tauriInvoke("backup_save_destination", {
        destination: { id: "", name, destination: { type: "local", path, alternate_paths: [], marker_file: null }, created_at: "" } satisfies SavedDestination,
      });
      setName("");
      setPath("");
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };
  const remove = async (id: string) => {
    try {
      await tauriInvoke("backup_delete_destination", { id });
      await load();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <Section title="Destination library" hint="Named destinations jobs can share (each job may use its own sub-folder).">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      <ul className="text-xs divide-y divide-zinc-100 dark:divide-zinc-800" data-testid="backup-destination-library">
        {items.length === 0 && <li className="py-1 text-zinc-500">No saved destinations yet.</li>}
        {items.map((d) => (
          <li key={d.id} className="flex items-center gap-2 py-1">
            <span className="font-medium">{d.name}</span>
            <span className="text-zinc-500 truncate flex-1">
              {d.destination.type === "local" ? d.destination.path : `connection ${d.destination.connection_id}:${d.destination.remote_path}`}
            </span>
            <Btn tone="danger" onClick={() => void remove(d.id)}>
              Remove
            </Btn>
          </li>
        ))}
      </ul>
      <div className="grid grid-cols-1 sm:grid-cols-[10rem_1fr_auto] gap-2 items-end">
        <Field label="Name">
          <input className={inputCls} value={name} onChange={(e) => setName(e.target.value)} data-testid="library-name" />
        </Field>
        <Field label="Folder">
          <PathInput value={path} onChange={setPath} placeholder="/Volumes/Backup" testId="library-path" />
        </Field>
        <Btn tone="primary" disabled={!name.trim() || !path.trim()} onClick={() => void add()} testId="library-add">
          Add
        </Btn>
      </div>
    </Section>
  );
}

export function BackupSettingsView() {
  return (
    <div className="p-3 space-y-3" data-testid="backup-settings">
      <DestinationLibrary />
      <DefaultsEditor />
    </div>
  );
}
