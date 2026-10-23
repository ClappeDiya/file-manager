/**
 * Notification profiles (K101):
 * - `NotificationProfilesManager` (Settings): list, add, edit, test and delete
 *   reusable channels. Standard: name, channel, secret, test. Advanced:
 *   minimum severity, message and HTML templates, webhook headers and body.
 * - `NotificationProfilePicker` (job editor, Standard): which profiles a job
 *   also sends to.
 * Organization profiles (from the console) are listed read-only.
 */
import { useEffect, useState } from "react";
import { inputCls } from "@/lib/backup-helpers";
import { errorText, type BackupNotification } from "@/stores/backup-store";
import { channelLabel, newProfile, profileSendsOn, useNotifyStore, type NotificationProfile, type Severity } from "@/stores/backup-notify";
import { AddChannel, NotificationCard } from "./backup-notifications";
import { AdvancedOnly } from "./advanced-only";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";

function ProfileEditor({ profile, onDone }: { profile: NotificationProfile; onDone: (msg: string | null) => void }) {
  const saveProfile = useNotifyStore((s) => s.saveProfile);
  const testProfile = useNotifyStore((s) => s.testProfile);
  const [p, setP] = useState(profile);
  const [err, setErr] = useState<string | null>(null);
  const save = async () => {
    setErr(null);
    try {
      const saved = await saveProfile(p);
      onDone(`Saved ${saved.name}`);
    } catch (e) {
      setErr(errorText(e));
    }
  };
  return (
    <div className="space-y-2 border border-zinc-200 dark:border-zinc-700 rounded p-2" data-testid="profile-editor">
      <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
        <Field label="Name">
          <input className={inputCls} value={p.name} onChange={(e) => setP({ ...p, name: e.target.value })} data-testid="profile-name" />
        </Field>
        <Field label="Description">
          <input className={inputCls} value={p.description} onChange={(e) => setP({ ...p, description: e.target.value })} />
        </Field>
      </div>
      <AdvancedOnly>
        <Field label="Minimum severity" hint="Send results at least this severe (replaces the per-result checkboxes).">
          <select className={inputCls} value={p.min_severity ?? ""} onChange={(e) => setP({ ...p, min_severity: (e.target.value || null) as Severity | null })} data-testid="profile-severity">
            <option value="">Use the checkboxes below</option>
            <option value="success">Every run (success and up)</option>
            <option value="warning">Warnings and errors</option>
            <option value="error">Errors only</option>
          </select>
        </Field>
      </AdvancedOnly>
      {/* Saved profiles test through the engine with their stored secrets. */}
      <NotificationCard n={p.notification} onChange={(notification) => setP({ ...p, notification })} onTest={p.id ? () => testProfile(p.id) : undefined} />
      {err && <Banner tone="error">{err}</Banner>}
      <div className="flex gap-1">
        <Btn tone="primary" testId="profile-save" onClick={save}>
          Save profile
        </Btn>
        <Btn onClick={() => onDone(null)}>Cancel</Btn>
      </div>
    </div>
  );
}

export function NotificationProfilesManager() {
  const { profiles, error, loadProfiles, deleteProfile, testProfile } = useNotifyStore();
  const [edit, setEdit] = useState<NotificationProfile | null>(null);
  const [adding, setAdding] = useState(false);
  const [msg, setMsg] = useState<{ tone: "error" | "success" | "warning"; text: string } | null>(null);
  const [confirmForce, setConfirmForce] = useState<{ id: string; text: string } | null>(null);

  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);

  const run = async (fn: () => Promise<string | void>) => {
    setMsg(null);
    try {
      const text = await fn();
      if (text) setMsg({ tone: "success", text });
    } catch (e) {
      setMsg({ tone: "error", text: errorText(e) });
    }
  };
  const remove = async (p: NotificationProfile, force: boolean) => {
    setMsg(null);
    try {
      const r = await deleteProfile(p.id, force);
      setConfirmForce(null);
      setMsg(r.warnings.length ? { tone: "warning", text: `Deleted ${p.name}. ${r.warnings.join(". ")}` } : { tone: "success", text: `Deleted ${p.name}` });
    } catch (e) {
      // Referenced: offer to delete anyway (removes the references).
      setConfirmForce({ id: p.id, text: errorText(e) });
    }
  };

  return (
    <Section title="Notification profiles" hint="Channels set up once and chosen in any job (and in policy templates). A job sends to its own channels plus its profiles; the same channel only once.">
      {(error || msg) && (
        <Banner tone={msg?.tone ?? "error"} onClose={() => setMsg(null)}>
          {msg?.text ?? error}
        </Banner>
      )}
      <ul className="text-xs divide-y divide-zinc-200 dark:divide-zinc-700" data-testid="profiles-list">
        {profiles.map((p) => (
          <li key={p.id} className="py-1 space-y-1">
            <div className="flex items-center gap-2">
              <span className="font-medium">{p.name}</span>
              <span className="px-1.5 py-0.5 rounded bg-zinc-100 dark:bg-zinc-800 text-[10px]">{channelLabel(p.notification)}</span>
              {p.managed && <span className="px-1.5 py-0.5 rounded text-[10px] bg-red-100 text-red-700 dark:bg-red-900/40 dark:text-red-300">Organization</span>}
              <span className="text-zinc-500 truncate flex-1">{profileSendsOn(p).join(", ")}</span>
              <Btn testId={`profile-test-${p.id}`} onClick={() => run(async () => { await testProfile(p.id); return `Test message sent through ${p.name}`; })}>
                Test
              </Btn>
              {!p.managed && <Btn onClick={() => setEdit(p)}>Edit</Btn>}
              {!p.managed && (
                <Btn tone="danger" testId={`profile-delete-${p.id}`} onClick={() => remove(p, false)}>
                  Delete
                </Btn>
              )}
            </div>
            {confirmForce?.id === p.id && (
              <Banner tone="warning" onClose={() => setConfirmForce(null)}>
                {confirmForce.text}{" "}
                <Btn tone="danger" testId="profile-delete-force" onClick={() => remove(p, true)}>
                  Delete anyway and remove it from them
                </Btn>
              </Banner>
            )}
          </li>
        ))}
        {profiles.length === 0 && <li className="py-1 text-zinc-500">No notification profiles yet.</li>}
      </ul>
      {!edit && !adding && (
        <Btn testId="profile-new" onClick={() => setAdding(true)}>
          New profile
        </Btn>
      )}
      {adding && (
        <div className="space-y-1" data-testid="profile-add">
          <span className="text-xs">Choose the channel:</span>
          <AddChannel
            onAdd={(n: BackupNotification) => {
              setAdding(false);
              setEdit(newProfile(n));
            }}
          />
          <Btn onClick={() => setAdding(false)}>Cancel</Btn>
        </div>
      )}
      {edit && (
        <ProfileEditor
          key={edit.id || "new"}
          profile={edit}
          onDone={(text) => {
            setEdit(null);
            if (text) setMsg({ tone: "success", text });
          }}
        />
      )}
    </Section>
  );
}

/** Job editor: the profiles this job also sends to. */
export function NotificationProfilePicker({ value, onChange }: { value: string[]; onChange: (ids: string[]) => void }) {
  const { profiles, loadProfiles } = useNotifyStore();
  useEffect(() => {
    void loadProfiles();
  }, [loadProfiles]);
  const missing = value.filter((id) => !profiles.some((p) => p.id === id));
  return (
    <Section title="Notification profiles" hint="Shared channels from Settings → Notification profiles.">
      {profiles.length === 0 && <p className="text-xs text-zinc-500">No profiles yet; create them in Backup settings.</p>}
      <div className="flex gap-3 flex-wrap" data-testid="profile-picker">
        {profiles.map((p) => (
          <Check
            key={p.id}
            label={`${p.name} (${channelLabel(p.notification)}${p.managed ? ", organization" : ""})`}
            checked={value.includes(p.id)}
            onChange={(v) => onChange(v ? [...value, p.id] : value.filter((x) => x !== p.id))}
          />
        ))}
      </div>
      {missing.length > 0 && profiles.length > 0 && (
        <Banner tone="warning">
          {missing.length} chosen profile(s) no longer exist.{" "}
          <Btn onClick={() => onChange(value.filter((id) => !missing.includes(id)))}>Remove them</Btn>
        </Banner>
      )}
    </Section>
  );
}
