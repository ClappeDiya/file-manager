/**
 * Repository safety in the maintenance view: who holds the repository
 * lock ("Repository in use by laptop (backup) since …", Standard), a
 * guarded Break lock (Advanced, with confirmation), the read-only state
 * and the one-click "Encrypt local database" migration (Standard).
 */
import { useCallback, useEffect, useState } from "react";
import { errorText, lockSummary, useBackupStore, type BackupJob, type RepoLockStatus } from "@/stores/backup-store";
import { Banner, Btn, Section } from "./backup-ui";
import { AdvancedOnly } from "./advanced-only";

export function BackupRepositorySafety({ job }: { job: BackupJob }) {
  const { lockStatus, breakLock, setDatabaseEncryption } = useBackupStore();
  const [status, setStatus] = useState<RepoLockStatus | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [info, setInfo] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirm, setConfirm] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await lockStatus(job.id));
      setError(null);
    } catch (e) {
      setError(errorText(e));
    }
  }, [job.id, lockStatus]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const act = async (f: () => Promise<string>) => {
    setBusy(true);
    setError(null);
    try {
      setInfo(await f());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const summary = lockSummary(status);
  const live = (status?.locks ?? []).filter((l) => !l.stale);
  const stale = (status?.locks ?? []).filter((l) => l.stale);
  const dbEncrypted = job.encrypt_local_database === true;

  return (
    <Section title="Repository safety">
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {info && <Banner tone="success" onClose={() => setInfo(null)}>{info}</Banner>}
      <div className="flex items-center gap-2 text-xs" data-testid="backup-lock-status">
        {status === null ? (
          <span className="text-zinc-500">Checking the repository lock…</span>
        ) : summary ? (
          <span className="text-amber-700 dark:text-amber-300">
            {summary}
            {live[0] && ` (lock expires ${new Date(live[0].expires_at).toLocaleString()})`}
          </span>
        ) : (
          <span className="text-zinc-600 dark:text-zinc-400">Not in use by another program.</span>
        )}
        <Btn disabled={busy} onClick={() => void refresh()} testId="backup-lock-refresh">
          Refresh
        </Btn>
      </div>
      {stale.length > 0 && (
        <p className="text-[11px] text-zinc-500">
          {stale.length} expired lock(s) from a program that stopped; the next operation takes them over.
        </p>
      )}
      {job.read_only && (
        <p className="text-xs text-sky-700 dark:text-sky-300" data-testid="backup-read-only-note">
          Read-only connection: restore, browse and verify only. Nothing is written to the destination.
        </p>
      )}

      <AdvancedOnly>
        {(live.length > 0 || stale.length > 0) && !job.read_only && (
          <div className="space-y-1">
            {!confirm ? (
              <Btn tone="danger" disabled={busy} onClick={() => setConfirm(true)} testId="backup-break-lock">
                Break lock…
              </Btn>
            ) : (
              <Banner tone="warning">
                <p>
                  Only break the lock when {live[0] ? `${live[0].hostname || "that computer"} is no longer running ${live[0].operation}` : "its program is no longer running"}. Two programs
                  changing one repository at the same time can damage it.
                </p>
                <div className="flex gap-1 mt-1">
                  <Btn
                    tone="danger"
                    disabled={busy}
                    testId="backup-break-lock-confirm"
                    onClick={() =>
                      void act(async () => {
                        const r = await breakLock(job.id, live.length === 1 ? live[0].owner_id : null);
                        setConfirm(false);
                        await refresh();
                        return `Removed ${r.removed.length} lock file(s).`;
                      })
                    }
                  >
                    Yes, break the lock
                  </Btn>
                  <Btn onClick={() => setConfirm(false)}>Cancel</Btn>
                </div>
              </Banner>
            )}
          </div>
        )}
      </AdvancedOnly>

      <div className="flex items-center gap-2 text-xs">
        <span className="text-zinc-600 dark:text-zinc-400" data-testid="backup-db-encryption">
          Local database: {dbEncrypted ? "encrypted (key in this computer's keychain)" : "not encrypted"}
        </span>
        {!dbEncrypted ? (
          <Btn
            tone="primary"
            disabled={busy}
            testId="backup-encrypt-db"
            title="Encrypts file names and settings in this job's local cache; the key is kept in the keychain"
            onClick={() => void act(async () => ((await setDatabaseEncryption(job.id, true)).encrypted ? "The local database is now encrypted." : "Nothing changed."))}
          >
            Encrypt local database
          </Btn>
        ) : (
          <AdvancedOnly>
            <Btn disabled={busy} testId="backup-decrypt-db" onClick={() => void act(async () => ((await setDatabaseEncryption(job.id, false)).encrypted ? "Nothing changed." : "The local database is no longer encrypted."))}>
              Store unencrypted
            </Btn>
          </AdvancedOnly>
        )}
      </div>
    </Section>
  );
}
