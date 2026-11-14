/**
 * In-app rclone remote manager: list remotes (with type), create one from
 * any backend the installed rclone offers (a form generated from its
 * schema, with a curated "Popular" list), edit, test, delete, finish
 * OAuth sign-ins in the browser, and add a remote as a connection.
 * Creation follows rclone's non-interactive state machine: questions
 * rclone asks are shown one at a time.
 */
import { useCallback, useEffect, useMemo, useState } from "react";
import { PasswordInput } from "@/components/password-input";
import { RcloneSchemaForm } from "@/components/rclone/rclone-schema-form";
import { CryptNamesDialog } from "@/components/rclone/rclone-crypt";
import { confirmText } from "@/lib/prompt-dialog";
import { errorText } from "@/lib/error-text";
import {
  cancelRcloneSignIn,
  changedParams,
  continueRcloneConfig,
  createRcloneRemote,
  deleteRcloneRemote,
  findProvider,
  getRcloneRemote,
  isOAuthQuestion,
  isRcloneUnavailable,
  isSecret,
  listRcloneRemotes,
  missingRequired,
  notifyRemotesChanged,
  onRcloneAuthUrl,
  popularBackends,
  rcloneProviders,
  signInRcloneRemote,
  testRcloneRemote,
  updateRcloneRemote,
  visibleOptions,
  type RcloneConfigStep,
  type RcloneObscureMode,
  type RcloneProvider,
  type RcloneRemote,
  type RcloneRemoteDetails,
  type RcloneRemoteTest,
} from "@/lib/rclone";
import { useUIStore } from "@/stores/ui-store";

const input = "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";
const btn = "text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-700 disabled:opacity-50";
const primary = "text-xs px-3 py-1 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50";

type Mode =
  | { kind: "list" }
  | { kind: "create"; type: string | null }
  | { kind: "edit"; name: string }
  | {
      kind: "question";
      name: string;
      type: string;
      step: RcloneConfigStep;
      /** The create/update call's params and obscure mode, re-sent with every answer. */
      params: Record<string, string>;
      obscure: RcloneObscureMode;
      created: boolean;
      manual: boolean;
    }
  | { kind: "signing-in"; name: string; type: string; url: string | null }
  | { kind: "done"; name: string; type: string; created: boolean };

const bare = (name: string) => name.replace(/:$/, "");

export function RcloneRemoteManager({
  onAddConnection,
  initialType = null,
  startInCreate = false,
  onClose,
}: {
  /** "Add as connection": the remote (`name:`) and a suggested connection name. */
  onAddConnection?: (remote: string, name: string) => void;
  /** Preselect a backend when opening in create mode. */
  initialType?: string | null;
  startInCreate?: boolean;
  onClose?: () => void;
}) {
  const advanced = useUIStore((s) => s.appMode === "advanced");
  const [remotes, setRemotes] = useState<RcloneRemote[] | null>(null);
  const [providers, setProviders] = useState<RcloneProvider[] | null>(null);
  /** rclone itself failed (list or providers): the message to show above everything. */
  const [loadError, setLoadError] = useState<string | null>(null);
  const [providersFailed, setProvidersFailed] = useState(false);
  const [mode, setMode] = useState<Mode>(startInCreate || initialType ? { kind: "create", type: initialType } : { kind: "list" });
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tests, setTests] = useState<Record<string, RcloneRemoteTest | "running">>({});
  const [cryptFor, setCryptFor] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setRemotes(await listRcloneRemotes());
      setLoadError(null);
    } catch (e) {
      setRemotes(null);
      setLoadError(errorText(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
    rcloneProviders().then(
      (p) => {
        setProviders(p ?? []);
        setProvidersFailed(false);
      },
      (e) => {
        setProvidersFailed(true);
        setLoadError((prev) => prev ?? errorText(e));
      },
    );
  }, [refresh]);

  // The sign-in address, as soon as rclone prints it.
  useEffect(() => {
    let off: (() => void) | null = null;
    let live = true;
    void onRcloneAuthUrl((e) => setMode((m) => (m.kind === "signing-in" && m.name === e.name ? { ...m, url: e.url } : m))).then((u) => {
      if (live) off = u;
      else u();
    });
    return () => {
      live = false;
      off?.();
    };
  }, []);

  const changed = useCallback(() => {
    notifyRemotesChanged();
    void refresh();
  }, [refresh]);

  /**
   * Show rclone's next step. `created` = the remote is new in this flow (it
   * is carried through every follow-up question, so the done screen still
   * says "created" and offers Add as connection); `first` = this is the
   * reply to the create call itself, the moment the remote appears in
   * rclone.conf. `params`/`obscure` are the create/update call's: follow-up
   * steps re-send the params, so they must keep its obscure mode (rclone
   * would otherwise guess, storing a base64-looking password raw).
   */
  const handleStep = useCallback(
    (name: string, type: string, step: RcloneConfigStep, params: Record<string, string>, obscure: RcloneObscureMode, created: boolean, first = false) => {
      if (step.done) {
        setMode({ kind: "done", name, type, created });
        changed();
      } else {
        setMode({ kind: "question", name, type, step, params, obscure, created, manual: false });
        if (first) changed();
      }
    },
    [changed],
  );

  const run = async (f: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await f();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const typeOf = (name: string) => remotes?.find((r) => bare(r.name) === name)?.backend ?? "";

  return (
    <div className="p-3 space-y-3" data-testid="rclone-remote-manager">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-semibold">rclone remotes</h3>
        <div className="flex gap-1">
          {mode.kind === "list" ? (
            <button
              type="button"
              className={primary}
              onClick={() => setMode({ kind: "create", type: null })}
              disabled={!providers || providersFailed}
              title={providersFailed ? "rclone's storage types could not be loaded" : undefined}
            >
              + New remote
            </button>
          ) : (
            mode.kind !== "signing-in" && (
              <button type="button" className={btn} onClick={() => setMode({ kind: "list" })}>
                Back to remotes
              </button>
            )
          )}
          {onClose && (
            <button type="button" className={btn} onClick={onClose} aria-label="Close rclone remotes">
              Close
            </button>
          )}
        </div>
      </div>

      {loadError && (
        <div className="rounded border border-red-200 dark:border-red-800 bg-red-50 dark:bg-red-900/20 p-2 space-y-1" role="alert" data-testid="rclone-load-error">
          {isRcloneUnavailable(loadError) && <p className="text-sm font-medium">rclone is not installed or not working — Settings → rclone</p>}
          <p className="text-xs text-red-600 dark:text-red-400">{loadError}</p>
        </div>
      )}

      {error && (
        <p className="text-xs text-red-600 dark:text-red-400" role="alert">
          {error}
        </p>
      )}

      {mode.kind === "list" && !(remotes === null && loadError) && (
        <RemoteList
          remotes={remotes}
          tests={tests}
          advanced={advanced}
          onTest={(name) => {
            setTests((t) => ({ ...t, [name]: "running" }));
            testRcloneRemote(name).then(
              (r) => setTests((t) => ({ ...t, [name]: r })),
              (e) => setTests((t) => ({ ...t, [name]: { ok: false, entries: 0, error: errorText(e), millis: 0 } })),
            );
          }}
          onEdit={(name) => setMode({ kind: "edit", name })}
          onDelete={(name) =>
            void (async () => {
              if (!(await confirmText(`Delete the rclone remote "${name}" from rclone.conf? Files on the storage are not touched.`, "Delete", true))) return;
              await run(async () => {
                await deleteRcloneRemote(name);
                changed();
              });
            })()
          }
          onAddConnection={onAddConnection ? (name) => onAddConnection(`${name}:`, name) : undefined}
          onCryptNames={(name) => setCryptFor(name)}
        />
      )}

      {mode.kind === "create" && providers && (
        <CreateRemote
          providers={providers}
          type={mode.type}
          advanced={advanced}
          busy={busy}
          existing={(remotes ?? []).map((r) => bare(r.name))}
          onType={(type) => setMode({ kind: "create", type })}
          onCreate={(name, provider, params, obscure) =>
            run(async () => {
              const step = await createRcloneRemote(name, provider.name, params, obscure);
              handleStep(name, provider.name, step, params, obscure, true, true);
            })
          }
        />
      )}

      {mode.kind === "edit" && providers && (
        <EditRemote
          key={mode.name}
          name={mode.name}
          providers={providers}
          advanced={advanced}
          busy={busy}
          onSave={(params, obscure) =>
            run(async () => {
              const step = await updateRcloneRemote(mode.name, params, obscure);
              handleStep(mode.name, typeOf(mode.name), step, params, obscure, false);
            })
          }
        />
      )}

      {mode.kind === "question" && (
        <Question
          mode={mode}
          busy={busy}
          onManual={() => setMode({ ...mode, manual: true })}
          onAnswer={(answer) =>
            run(async () => {
              const step = await continueRcloneConfig(mode.name, mode.step.state, answer, mode.params, mode.obscure);
              handleStep(mode.name, mode.type, step, mode.params, mode.obscure, mode.created);
            })
          }
          onSignIn={() =>
            void (async () => {
              setError(null);
              setMode({ kind: "signing-in", name: mode.name, type: mode.type, url: null });
              try {
                const step = await signInRcloneRemote(mode.name, mode.step.state, mode.params, mode.obscure);
                handleStep(mode.name, mode.type, step, mode.params, mode.obscure, mode.created);
              } catch (e) {
                setError(errorText(e));
                setMode(mode);
              }
            })()
          }
          onDiscard={() =>
            void run(async () => {
              await deleteRcloneRemote(mode.name);
              changed();
              setMode({ kind: "list" });
            })
          }
        />
      )}

      {mode.kind === "signing-in" && (
        <div className="rounded border border-blue-200 dark:border-blue-800 bg-blue-50 dark:bg-blue-900/20 p-3 space-y-2" data-testid="rclone-signing-in">
          <p className="text-sm">Finish signing in to {mode.type} in your browser. This window updates when you are done.</p>
          {mode.url ? (
            <p className="text-xs">
              Browser did not open?{" "}
              <a href={mode.url} target="_blank" rel="noreferrer" className="text-blue-600 dark:text-blue-400 underline break-all">
                Open the sign-in page
              </a>
            </p>
          ) : (
            <p className="text-xs text-gray-500">Starting rclone's sign-in…</p>
          )}
          <button type="button" className={btn} onClick={() => void cancelRcloneSignIn(mode.name)}>
            Cancel sign-in
          </button>
        </div>
      )}

      {mode.kind === "done" && (
        <div className="rounded border border-green-200 dark:border-green-800 bg-green-50 dark:bg-green-900/20 p-3 space-y-2" data-testid="rclone-done">
          <p className="text-sm">
            Remote <span className="font-mono">{mode.name}:</span> {mode.created ? "was created" : "was updated"}.
          </p>
          <div className="flex flex-wrap gap-1">
            {onAddConnection && mode.created && (
              <button type="button" className={primary} onClick={() => onAddConnection(`${mode.name}:`, mode.name)}>
                Add as connection
              </button>
            )}
            <button
              type="button"
              className={btn}
              onClick={() => {
                const name = mode.name;
                setTests((t) => ({ ...t, [name]: "running" }));
                testRcloneRemote(name).then(
                  (r) => setTests((t) => ({ ...t, [name]: r })),
                  (e) => setTests((t) => ({ ...t, [name]: { ok: false, entries: 0, error: errorText(e), millis: 0 } })),
                );
              }}
            >
              Test
            </button>
            <button type="button" className={btn} onClick={() => setMode({ kind: "list" })}>
              Done
            </button>
          </div>
          <TestResult result={tests[mode.name]} />
        </div>
      )}

      {cryptFor && <CryptNamesDialog remote={`${cryptFor}:`} onClose={() => setCryptFor(null)} />}
    </div>
  );
}

function TestResult({ result }: { result: RcloneRemoteTest | "running" | undefined }) {
  if (!result) return null;
  if (result === "running") return <p className="text-xs text-gray-500">Testing…</p>;
  return result.ok ? (
    <p className="text-xs text-green-600 dark:text-green-400" data-testid="rclone-test-ok">
      Works: {result.entries} item{result.entries === 1 ? "" : "s"} at the top ({result.millis} ms)
    </p>
  ) : (
    <p className="text-xs text-red-600 dark:text-red-400" data-testid="rclone-test-failed">
      Failed: {result.error}
    </p>
  );
}

function RemoteList({
  remotes,
  tests,
  advanced,
  onTest,
  onEdit,
  onDelete,
  onAddConnection,
  onCryptNames,
}: {
  remotes: RcloneRemote[] | null;
  tests: Record<string, RcloneRemoteTest | "running">;
  advanced: boolean;
  onTest: (name: string) => void;
  onEdit: (name: string) => void;
  onDelete: (name: string) => void;
  onAddConnection?: (name: string) => void;
  onCryptNames: (name: string) => void;
}) {
  if (remotes === null) return <p className="text-xs text-gray-400">Loading…</p>;
  if (remotes.length === 0)
    return <p className="text-xs text-gray-500">No rclone remotes yet. Create one with “New remote”; it is saved in rclone.conf.</p>;
  return (
    <ul className="divide-y divide-gray-200 dark:divide-gray-700 border border-gray-200 dark:border-gray-700 rounded">
      {remotes.map((r) => {
        const name = bare(r.name);
        return (
          <li key={r.name} className="px-2 py-1.5 space-y-1" data-testid={`rclone-remote-${name}`}>
            <div className="flex items-center gap-2">
              <span className="font-mono text-sm truncate">{r.name}</span>
              <span className="text-[10px] uppercase text-gray-400">{r.backend}</span>
              {r.backend === "crypt" && <span className="text-[10px] rounded bg-amber-100 dark:bg-amber-900/40 px-1">encrypted</span>}
              <div className="ml-auto flex gap-1">
                <button type="button" className={btn} onClick={() => onTest(name)}>
                  Test
                </button>
                <button type="button" className={btn} onClick={() => onEdit(name)}>
                  Edit
                </button>
                {onAddConnection && (
                  <button type="button" className={btn} onClick={() => onAddConnection(name)}>
                    Add as connection
                  </button>
                )}
                {advanced && r.backend === "crypt" && (
                  <button type="button" className={btn} onClick={() => onCryptNames(name)} title="Encrypt or decrypt file names offline">
                    Names…
                  </button>
                )}
                <button type="button" className={`${btn} text-red-600`} onClick={() => onDelete(name)}>
                  Delete
                </button>
              </div>
            </div>
            <TestResult result={tests[name]} />
          </li>
        );
      })}
    </ul>
  );
}

function CreateRemote({
  providers,
  type,
  advanced,
  busy,
  existing,
  onType,
  onCreate,
}: {
  providers: RcloneProvider[];
  type: string | null;
  advanced: boolean;
  busy: boolean;
  existing: string[];
  onType: (type: string | null) => void;
  onCreate: (name: string, provider: RcloneProvider, params: Record<string, string>, obscure: RcloneObscureMode) => void;
}) {
  const provider = type ? findProvider(providers, type) : null;
  const [search, setSearch] = useState("");
  const [name, setName] = useState(provider?.prefix ?? "");
  const [values, setValues] = useState<Record<string, string>>({});
  const [alreadyObscured, setAlreadyObscured] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  const popular = useMemo(() => popularBackends(providers), [providers]);

  useEffect(() => {
    setName(provider ? provider.prefix.replace(/[^\w.+@-]/g, "") : "");
    setValues({});
    setProblem(null);
  }, [provider]);

  if (!provider) {
    const q = search.trim().toLowerCase();
    const list = providers.filter((p) => !p.hidden && (!q || p.name.toLowerCase().includes(q) || p.description.toLowerCase().includes(q) || p.prefix.includes(q)));
    return (
      <div className="space-y-2" data-testid="rclone-choose-type">
        <p className="text-xs font-medium">Popular</p>
        <div className="flex flex-wrap gap-1" data-testid="rclone-popular">
          {popular.map((b) => (
            <button
              key={b.type}
              type="button"
              className={btn}
              disabled={!b.provider}
              title={b.provider ? b.provider.description : "Not available in the installed rclone; update rclone (Settings → rclone)"}
              onClick={() => b.provider && onType(b.provider.name)}
            >
              {b.label}
            </button>
          ))}
        </div>
        <label className="block text-xs font-medium" htmlFor="rclone-type-search">
          All storage types ({list.length})
        </label>
        <input id="rclone-type-search" value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search, e.g. sftp, s3, drive" className={input} />
        <ul className="max-h-64 overflow-y-auto border border-gray-200 dark:border-gray-700 rounded divide-y divide-gray-100 dark:divide-gray-800">
          {list.map((p) => (
            <li key={p.name}>
              <button type="button" className="w-full text-left px-2 py-1 text-xs hover:bg-gray-100 dark:hover:bg-gray-800" onClick={() => onType(p.name)}>
                <span className="font-mono">{p.name}</span> <span className="text-gray-500">— {p.description}</span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    );
  }

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const n = name.trim().replace(/:$/, "");
    if (!n) return setProblem("Name the remote.");
    if (existing.includes(n)) return setProblem(`A remote named "${n}" already exists.`);
    const options = visibleOptions(provider, values, advanced);
    const missing = missingRequired(options, values);
    if (missing.length) return setProblem(`Fill in: ${missing.map((o) => o.name).join(", ")}`);
    setProblem(null);
    onCreate(n, provider, changedParams(options, values), alreadyObscured ? "already_obscured" : "obscure");
  };

  return (
    <form className="space-y-2" onSubmit={submit} data-testid="rclone-create-form">
      <div className="flex items-center gap-2">
        <span className="text-xs">
          Type: <span className="font-mono">{provider.name}</span> <span className="text-gray-500">— {provider.description}</span>
        </span>
        <button type="button" className={`${btn} ml-auto`} onClick={() => onType(null)}>
          Change
        </button>
      </div>
      <div>
        <label className="block text-xs font-medium mb-1" htmlFor="rclone-new-name">
          Remote name
        </label>
        <input id="rclone-new-name" value={name} onChange={(e) => setName(e.target.value)} className={`${input} font-mono`} />
      </div>
      <RcloneSchemaForm provider={provider} values={values} onChange={setValues} advanced={advanced} />
      {advanced && (
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={alreadyObscured} onChange={(e) => setAlreadyObscured(e.target.checked)} />
          Passwords above are already obscured (<span className="font-mono">rclone obscure</span>); store them as typed
        </label>
      )}
      {problem && <p className="text-xs text-red-600 dark:text-red-400">{problem}</p>}
      <button type="submit" className={primary} disabled={busy}>
        {busy ? "Creating…" : "Create remote"}
      </button>
    </form>
  );
}

function EditRemote({
  name,
  providers,
  advanced,
  busy,
  onSave,
}: {
  name: string;
  providers: RcloneProvider[];
  advanced: boolean;
  busy: boolean;
  onSave: (params: Record<string, string>, obscure: RcloneObscureMode) => void;
}) {
  const [details, setDetails] = useState<RcloneRemoteDetails | null>(null);
  const [values, setValues] = useState<Record<string, string>>({});
  const [alreadyObscured, setAlreadyObscured] = useState(false);
  const [problem, setProblem] = useState<string | null>(null);
  useEffect(() => {
    getRcloneRemote(name).then(
      (d) => {
        setDetails(d);
        setValues(d.values);
      },
      (e) => setProblem(errorText(e)),
    );
  }, [name]);
  if (!details) return <p className="text-xs text-gray-400">{problem ?? "Loading…"}</p>;
  const provider = findProvider(providers, details.type);
  if (!provider) return <p className="text-xs text-red-600">This rclone has no “{details.type}” backend; edit the remote with `rclone config`.</p>;
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const options = visibleOptions(provider, values, advanced);
    const missing = missingRequired(options, values, details.secrets_set);
    if (missing.length) return setProblem(`Fill in: ${missing.map((o) => o.name).join(", ")}`);
    const params = changedParams(options, values, details.values);
    if (Object.keys(params).length === 0) return setProblem("Nothing changed.");
    setProblem(null);
    onSave(params, alreadyObscured ? "already_obscured" : "obscure");
  };
  return (
    <form className="space-y-2" onSubmit={submit} data-testid="rclone-edit-form">
      <p className="text-xs">
        Editing <span className="font-mono">{name}:</span> ({details.type})
      </p>
      <RcloneSchemaForm provider={provider} values={values} onChange={setValues} advanced={advanced} secretsSet={details.secrets_set} />
      {advanced && (
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={alreadyObscured} onChange={(e) => setAlreadyObscured(e.target.checked)} />
          New passwords are already obscured
        </label>
      )}
      {problem && <p className="text-xs text-red-600 dark:text-red-400">{problem}</p>}
      <button type="submit" className={primary} disabled={busy}>
        {busy ? "Saving…" : "Save"}
      </button>
    </form>
  );
}

function Question({
  mode,
  busy,
  onAnswer,
  onSignIn,
  onManual,
  onDiscard,
}: {
  mode: Extract<Mode, { kind: "question" }>;
  busy: boolean;
  onAnswer: (answer: string) => void;
  onSignIn: () => void;
  onManual: () => void;
  onDiscard: () => void;
}) {
  const o = mode.step.option;
  const [answer, setAnswer] = useState(o?.default ?? "");
  useEffect(() => setAnswer(mode.step.option?.default ?? ""), [mode.step]);
  const oauth = isOAuthQuestion(mode.step) && !mode.manual;
  return (
    <div className="space-y-2 rounded border border-gray-200 dark:border-gray-700 p-3" data-testid="rclone-question">
      {mode.step.error && <p className="text-xs text-red-600 dark:text-red-400">{mode.step.error}</p>}
      {oauth ? (
        <>
          <p className="text-sm">
            <span className="font-mono">{mode.name}:</span> signs in through your browser.
          </p>
          <div className="flex flex-wrap gap-1">
            <button type="button" className={primary} onClick={onSignIn} disabled={busy}>
              Sign in with browser
            </button>
            <button type="button" className={btn} onClick={onManual}>
              Answer rclone's questions manually
            </button>
          </div>
        </>
      ) : o ? (
        <form
          className="space-y-2"
          onSubmit={(e) => {
            e.preventDefault();
            onAnswer(answer);
          }}
        >
          <label className="block text-xs font-medium" htmlFor="rclone-answer">
            {o.name}
            {o.required && <span className="text-red-500"> *</span>}
          </label>
          <p className="text-xs text-gray-600 dark:text-gray-300 whitespace-pre-wrap">{o.help}</p>
          {o.examples.length > 0 && o.exclusive ? (
            <select id="rclone-answer" value={answer} onChange={(e) => setAnswer(e.target.value)} className={input}>
              {o.examples.map((ex) => (
                <option key={ex.value} value={ex.value}>
                  {ex.value} {ex.help ? `— ${ex.help.split("\n")[0]}` : ""}
                </option>
              ))}
            </select>
          ) : isSecret(o) ? (
            <PasswordInput id="rclone-answer" value={answer} onChange={(e) => setAnswer(e.target.value)} className={input} />
          ) : (
            <input id="rclone-answer" value={answer} onChange={(e) => setAnswer(e.target.value)} className={`${input} font-mono`} />
          )}
          <button type="submit" className={primary} disabled={busy || (o.required && !answer)}>
            {busy ? "Sending…" : "Continue"}
          </button>
        </form>
      ) : (
        <p className="text-xs">rclone is waiting for more input but sent no question; finish with `rclone config`.</p>
      )}
      {mode.created && (
        <button type="button" className={`${btn} text-red-600`} onClick={onDiscard} disabled={busy}>
          Discard this remote
        </button>
      )}
    </div>
  );
}
