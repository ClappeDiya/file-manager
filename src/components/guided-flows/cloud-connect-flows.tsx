/**
 * Cloud Connection Guided Flows (T-056)
 *
 * Wizards 1-4:
 * 1. Connect OneDrive (personal, or work/school with SharePoint libraries)
 * 2. Connect Google Drive (My Drive, Shared with me, shared drives)
 * 3. Connect Dropbox (own folder, team space, team folders)
 * 4. Connect GCS (S3-compatible, HMAC keys)
 *
 * Every step does the real work: the connection is saved, the browser
 * sign-in runs through the OAuth commands (`src/lib/oauth.ts`), the
 * location is picked from the account's real drives / sites / team spaces,
 * and the last step connects and lists it. Nothing reports success it has
 * not checked.
 */
import { useCallback, useState } from "react";
import { useUIStore } from "@/stores/ui-store";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { announceConnectionsChanged, type OAuthSignedIn } from "@/lib/oauth";
import { OAuthSignIn } from "@/components/oauth-sign-in";
import { CloudScopeFields, type CloudExtras, type OAuthSettings } from "@/components/cloud-connection-fields";
import { PasswordInput } from "@/components/password-input";
import { WizardShell, type WizardStepDef } from "./wizard-shell";
import { cn } from "@ufop/ui-components";
import { Cloud, Check, FolderOpen, Shield, Key, Loader2 } from "lucide-react";

// ──────────────────────────────────────────────
// Shared cloud connect wizard logic
// ──────────────────────────────────────────────

type CloudProvider = "onedrive" | "gdrive" | "dropbox";

interface RemoteEntry {
  name: string;
  path: string;
  is_dir: boolean;
  size: number;
}

/** What `save_connection` returns (the fields used here). */
interface SavedProfile {
  id: string;
  name: string;
  protocol: string;
}

const PROVIDER_INFO: Record<CloudProvider, { name: string; protocol: string; description: string }> = {
  onedrive: {
    name: "Microsoft OneDrive",
    protocol: "onedrive",
    description: "Connect your OneDrive (or a SharePoint document library) to browse and transfer files.",
  },
  gdrive: {
    name: "Google Drive",
    protocol: "google_drive",
    description: "Connect Google Drive to browse and transfer files. Shared drives are supported.",
  },
  dropbox: {
    name: "Dropbox",
    protocol: "dropbox",
    description: "Connect Dropbox to browse and transfer files, including team spaces and team folders.",
  },
};

const field = "w-full text-sm px-3 py-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]";
const primary = "px-6 py-3 rounded-xl bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] font-medium text-sm hover:opacity-90 transition-all disabled:opacity-50";

/** Save (create or update) a connection. */
async function saveProfile(args: Record<string, unknown>): Promise<SavedProfile> {
  return tauriInvoke<SavedProfile>("save_connection", args);
}

/** Connect a saved connection and list its root: the real test. */
async function connectAndList(protocol: string, connectionId: string, host = ""): Promise<RemoteEntry[]> {
  await tauriInvoke("connector_connect", { protocol, host, port: null, username: null, remotePath: "/", connectionId });
  return tauriInvoke<RemoteEntry[]>("connector_list_remote", { protocol, path: "/", connectionId });
}

function CloudConnectWizard({ provider }: { provider: CloudProvider }) {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);
  const info = PROVIDER_INFO[provider];

  const [step, setStep] = useState(0);
  const [name, setName] = useState(info.name);
  const [accountKind, setAccountKind] = useState<"personal" | "business">("personal");
  const [saved, setSaved] = useState<SavedProfile | null>(null);
  const [signedIn, setSignedIn] = useState<OAuthSignedIn | null>(null);
  const [scope, setScope] = useState<CloudExtras>({});
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [listing, setListing] = useState<RemoteEntry[] | null>(null);
  // The OAuth app the sign-in saved on the connection (your own, Advanced).
  const [clientId, setClientId] = useState<string | null>(null);

  const extras = useCallback(
    (): Record<string, unknown> => ({
      assume_role: null,
      rclone_remote: null,
      preset: null,
      swift: null,
      // Every save replaces the extras: carry the sign-in's OAuth app over,
      // or the next sign-in would fall back to the built-in one.
      oauth: { ...(scope.oauth ?? {}), client_id: clientId, account_kind: provider === "onedrive" ? scope.oauth?.account_kind ?? accountKind : "personal" },
      google_drive: provider === "gdrive" ? scope.google_drive ?? null : null,
      onedrive: provider === "onedrive" ? scope.onedrive ?? null : null,
      dropbox: provider === "dropbox" ? scope.dropbox ?? null : null,
    }),
    [scope, provider, accountKind, clientId],
  );

  const save = useCallback(async (): Promise<SavedProfile> => {
    const p = await saveProfile({
      connectionId: saved?.id ?? null,
      name: name.trim() || info.name,
      protocol: info.protocol,
      host: "",
      port: null,
      username: null,
      password: null,
      remotePath: "/",
      groupId: null,
      extras: extras(),
    });
    setSaved(p);
    return p;
  }, [saved, name, info, extras]);

  /** Signed in: read back the OAuth app the sign-in saved on the connection. */
  const handleSignedIn = useCallback(async (s: OAuthSignedIn) => {
    try {
      const p = await tauriInvoke<{ extras?: { oauth?: OAuthSettings | null } | null } | null>("get_connection", { connectionId: s.connection_id });
      setClientId(p?.extras?.oauth?.client_id ?? null);
    } catch (e) {
      console.error("Cannot read the connection back:", e);
    }
    setSignedIn(s);
  }, []);

  /** Save the chosen location, then connect and list it. */
  const preview = useCallback(async () => {
    setError(null);
    setBusy(true);
    setListing(null);
    try {
      const p = await save();
      setListing(await connectAndList(info.protocol, p.id));
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }, [save, info.protocol]);

  const steps: WizardStepDef[] = [
    {
      id: "account",
      label: "Account",
      helpText: `Name this connection${provider === "onedrive" ? " and choose the kind of Microsoft account" : ""}. It is saved when you continue.`,
      validate: () => (name.trim() ? null : "Enter a name for the connection."),
    },
    {
      id: "sign-in",
      label: "Sign In",
      helpText: `Sign in with ${info.name} in your browser. UFOP never sees your password; the access it is granted is kept in the system keychain.`,
      validate: () => (signedIn ? null : `Sign in to ${info.name} to continue.`),
    },
    {
      id: "location",
      label: "Location",
      helpText: "Choose which part of the account this connection opens, then check it by listing it.",
      validate: () => (listing ? null : "Check the location (List it) before finishing."),
    },
  ];

  const goTo = useCallback(
    async (next: number) => {
      setError(null);
      if (step === 0 && next === 1) {
        setBusy(true);
        try {
          await save();
        } catch (e) {
          setError(errorText(e));
          return;
        } finally {
          setBusy(false);
        }
      }
      setStep(next);
    },
    [step, save],
  );

  const handleComplete = useCallback(async () => {
    setBusy(true);
    try {
      await save();
      addActivity({
        type: "connect",
        summary: `Connected to ${info.name}${signedIn?.account ? ` as ${signedIn.account}` : ""}.`,
        paths: [],
        undoable: false,
      });
      announceConnectionsChanged();
      setActiveGuidedFlow(null);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  }, [save, addActivity, info.name, signedIn, setActiveGuidedFlow]);

  const handleCancel = useCallback(() => {
    // A connection saved along the way stays (it can be signed in later from Connections).
    if (saved) announceConnectionsChanged();
    setActiveGuidedFlow(null);
  }, [saved, setActiveGuidedFlow]);

  return (
    <WizardShell
      title={`Connect ${info.name}`}
      subtitle={info.description}
      steps={steps}
      currentStep={step}
      onStepChange={(i) => void goTo(i)}
      onComplete={() => void handleComplete()}
      onCancel={handleCancel}
      loading={busy}
      completeLabel="Finish"
    >
      {step === 0 && (
        <div className="space-y-4 py-2">
          <div className="flex items-center gap-3">
            <div className="h-12 w-12 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center">
              <Cloud className="h-6 w-6 text-[color:var(--color-primary)]" aria-hidden="true" />
            </div>
            <p className="text-sm text-[color:var(--color-text-secondary)]">{info.description}</p>
          </div>
          <div>
            <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1" htmlFor="cloud-name">Connection name</label>
            <input id="cloud-name" value={name} onChange={(e) => setName(e.target.value)} className={field} />
          </div>
          {provider === "onedrive" && (
            <div>
              <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1" htmlFor="cloud-account-kind">Account</label>
              <select id="cloud-account-kind" value={accountKind} onChange={(e) => setAccountKind(e.target.value as "personal" | "business")} className={field}>
                <option value="personal">Personal Microsoft account</option>
                <option value="business">Work or school (Microsoft 365, SharePoint)</option>
              </select>
            </div>
          )}
        </div>
      )}

      {step === 1 && saved && (
        <div className="py-2 space-y-3">
          {signedIn ? (
            <p className="text-sm flex items-center gap-2 text-[color:var(--color-text)]" data-testid="cloud-signed-in">
              <Check className="h-4 w-4 text-[color:var(--color-primary)]" aria-hidden="true" />
              Signed in{signedIn.account ? ` as ${signedIn.account}` : ""}.
            </p>
          ) : (
            <OAuthSignIn connectionId={saved.id} protocol={info.protocol} onSignedIn={(s) => void handleSignedIn(s)} />
          )}
          <p className="text-xs text-[color:var(--color-text-tertiary)]">
            <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
            OAuth 2.0 with PKCE; tokens are stored in the system keychain.
          </p>
        </div>
      )}

      {step === 2 && saved && (
        <div className="py-2 space-y-3">
          <CloudScopeFields
            protocol={info.protocol}
            connectionId={saved.id}
            value={{ ...scope, oauth: scope.oauth ?? { client_id: null, account_kind: accountKind } }}
            onChange={(next) => {
              setScope(next);
              setListing(null);
            }}
          />
          <button type="button" className={primary} disabled={busy} onClick={() => void preview()}>
            {busy ? <Loader2 className="h-4 w-4 animate-spin inline" aria-hidden="true" /> : "List it"}
          </button>
          {listing && (
            <div data-testid="cloud-listing">
              <p className="text-xs font-medium text-[color:var(--color-text-secondary)] mb-1">
                Connected. {listing.length} item{listing.length === 1 ? "" : "s"} at the top:
              </p>
              <ul className="max-h-40 overflow-y-auto text-sm space-y-0.5">
                {listing.slice(0, 50).map((e) => (
                  <li key={e.path} className="flex items-center gap-2">
                    {e.is_dir && <FolderOpen className="h-3.5 w-3.5" aria-hidden="true" />}
                    {e.name}
                  </li>
                ))}
              </ul>
            </div>
          )}
        </div>
      )}

      {error && (
        <p className="text-sm text-[color:var(--color-error)] mt-2" role="alert" data-testid="cloud-error">
          {error}
        </p>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// GCS (S3-Compatible) Connect Wizard
// ──────────────────────────────────────────────

interface GCSConnectState {
  accessKeyId: string;
  secretAccessKey: string;
  endpoint: string;
  testResult: "untested" | "testing" | "success" | "failed";
  error: string | null;
  buckets: number;
  connectionId: string | null;
}

function ConnectGCSWizardInner() {
  const addActivity = useUIStore((s) => s.addActivity);
  const setActiveGuidedFlow = useUIStore((s) => s.setActiveGuidedFlow);

  const [state, setState] = useState<GCSConnectState>({
    accessKeyId: "",
    secretAccessKey: "",
    endpoint: "https://storage.googleapis.com",
    testResult: "untested",
    error: null,
    buckets: 0,
    connectionId: null,
  });

  const [step, setStep] = useState(0);

  const steps: WizardStepDef[] = [
    {
      id: "select-preset",
      label: "Select Preset",
      helpText:
        "Google Cloud Storage is accessed via an S3-compatible endpoint. The endpoint URL is pre-filled for you.",
      validate: () => (state.endpoint.trim() ? null : "Enter the endpoint URL."),
    },
    {
      id: "enter-credentials",
      label: "Enter Credentials",
      helpText:
        "Enter your HMAC access key and secret. You can create these in the GCP Console under Cloud Storage > Settings > Interoperability.",
      validate: () =>
        state.accessKeyId && state.secretAccessKey
          ? null
          : "Please enter both your Access Key ID and Secret Access Key.",
    },
    {
      id: "test-connection",
      label: "Test Connection",
      helpText:
        "We save the connection and list your buckets through the S3-compatible endpoint to check the keys.",
      validate: () => (state.testResult === "success" ? null : "Test the connection first."),
    },
  ];

  const test = useCallback(async () => {
    setState((prev) => ({ ...prev, testResult: "testing", error: null }));
    try {
      const p = await saveProfile({
        connectionId: state.connectionId,
        name: "Google Cloud Storage",
        protocol: "s3",
        host: state.endpoint.trim(),
        port: null,
        username: state.accessKeyId.trim(),
        password: state.secretAccessKey,
        remotePath: "/",
        groupId: null,
      });
      const buckets = await connectAndList("s3", p.id, state.endpoint.trim());
      setState((prev) => ({ ...prev, connectionId: p.id, testResult: "success", buckets: buckets.length }));
    } catch (e) {
      setState((prev) => ({ ...prev, testResult: "failed", error: errorText(e) }));
    }
  }, [state.connectionId, state.endpoint, state.accessKeyId, state.secretAccessKey]);

  const handleComplete = useCallback(() => {
    addActivity({
      type: "connect",
      summary:
        "Connected to Google Cloud Storage (S3 Compatible) via HMAC keys.",
      paths: [],
      undoable: false,
    });
    announceConnectionsChanged();
    setActiveGuidedFlow(null);
  }, [addActivity, setActiveGuidedFlow]);

  const handleCancel = useCallback(() => {
    if (state.connectionId) announceConnectionsChanged();
    setActiveGuidedFlow(null);
  }, [state.connectionId, setActiveGuidedFlow]);

  return (
    <WizardShell
      title="Connect Google Cloud Storage"
      subtitle="Set up an S3-compatible connection to Google Cloud Storage using HMAC keys."
      steps={steps}
      currentStep={step}
      onStepChange={setStep}
      onComplete={handleComplete}
      onCancel={handleCancel}
      completeLabel="Connect"
    >
      {/* Step 1: Select Preset */}
      {step === 0 && (
        <div className="flex flex-col items-center text-center py-6">
          <div className="h-16 w-16 rounded-2xl bg-[var(--color-primary)]/10 flex items-center justify-center mb-6">
            <Cloud className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
          </div>
          <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
            Google Cloud Storage (S3 Compatible)
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4">
            GCS provides an S3-compatible API. The endpoint URL is pre-configured
            for you.
          </p>
          <div className="w-full max-w-sm text-left">
            <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1" htmlFor="gcs-endpoint">
              Endpoint URL
            </label>
            <input
              id="gcs-endpoint"
              type="text"
              value={state.endpoint}
              onChange={(e) =>
                setState((prev) => ({ ...prev, endpoint: e.target.value, testResult: "untested" }))
              }
              className={field}
            />
          </div>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-4 max-w-sm">
            <Shield className="h-3 w-3 inline mr-1" aria-hidden="true" />
            Uses S3-compatible HMAC authentication
          </p>
        </div>
      )}

      {/* Step 2: Enter Credentials */}
      {step === 1 && (
        <div>
          <h3 className="text-base font-semibold text-[color:var(--color-text)] mb-2">
            Enter your HMAC credentials
          </h3>
          <p className="text-sm text-[color:var(--color-text-secondary)] mb-4">
            Create HMAC keys in{" "}
            <span className="font-medium">
              GCP Console &gt; Cloud Storage &gt; Settings &gt; Interoperability
            </span>.
          </p>
          <div className="space-y-3">
            <div>
              <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1" htmlFor="gcs-key-id">
                Access Key ID
              </label>
              <div className="relative">
                <Key
                  className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[color:var(--color-text-tertiary)]"
                  aria-hidden="true"
                />
                <input
                  id="gcs-key-id"
                  type="text"
                  value={state.accessKeyId}
                  onChange={(e) =>
                    setState((prev) => ({
                      ...prev,
                      accessKeyId: e.target.value,
                      testResult: "untested",
                    }))
                  }
                  placeholder="GOOG1E..."
                  className="w-full text-sm pl-10 pr-3 py-2 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] text-[color:var(--color-text)]"
                />
              </div>
            </div>
            <div>
              <label className="block text-xs font-medium text-[color:var(--color-text-secondary)] mb-1" htmlFor="gcs-secret">
                Secret Access Key
              </label>
              <PasswordInput
                id="gcs-secret"
                value={state.secretAccessKey}
                onChange={(e) =>
                  setState((prev) => ({
                    ...prev,
                    secretAccessKey: e.target.value,
                    testResult: "untested",
                  }))
                }
                placeholder="Enter your secret key"
                className={field}
              />
            </div>
          </div>
          <p className="text-xs text-[color:var(--color-text-tertiary)] mt-3">
            Your credentials are stored securely in the system keychain and never
            sent to third parties.
          </p>
        </div>
      )}

      {/* Step 3: Test Connection */}
      {step === 2 && (
        <div className="flex flex-col items-center text-center py-6">
          <div
            className={cn(
              "h-16 w-16 rounded-2xl flex items-center justify-center mb-6",
              state.testResult === "success"
                ? "bg-[var(--color-primary)]/10"
                : state.testResult === "failed"
                  ? "bg-[var(--color-error)]/10"
                  : "bg-[var(--color-bg-secondary)]",
            )}
          >
            {state.testResult === "success" ? (
              <Check className="h-8 w-8 text-[color:var(--color-primary)]" aria-hidden="true" />
            ) : (
              <Cloud className="h-8 w-8 text-[color:var(--color-text-secondary)]" aria-hidden="true" />
            )}
          </div>

          {(state.testResult === "untested" || state.testResult === "testing") && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Ready to test your connection
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-6">
                We'll save the connection and list your buckets with your HMAC credentials.
              </p>
              <button className={primary} disabled={state.testResult === "testing"} onClick={() => void test()}>
                {state.testResult === "testing" ? "Testing…" : "Test Connection"}
              </button>
            </>
          )}

          {state.testResult === "success" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-text)] mb-2">
                Connection successful
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm" data-testid="gcs-success">
                Listed {state.buckets} bucket{state.buckets === 1 ? "" : "s"} through the S3-compatible endpoint.
              </p>
            </>
          )}

          {state.testResult === "failed" && (
            <>
              <h3 className="text-lg font-semibold text-[color:var(--color-error)] mb-2">
                Connection failed
              </h3>
              <p className="text-sm text-[color:var(--color-text-secondary)] max-w-sm mb-4" role="alert">
                {state.error}
              </p>
              <button className={primary} onClick={() => void test()}>
                Try Again
              </button>
            </>
          )}
        </div>
      )}
    </WizardShell>
  );
}

// ──────────────────────────────────────────────
// Exported Wizards
// ──────────────────────────────────────────────

export function ConnectOneDriveWizard() {
  return <CloudConnectWizard provider="onedrive" />;
}

export function ConnectGDriveWizard() {
  return <CloudConnectWizard provider="gdrive" />;
}

export function ConnectDropboxWizard() {
  return <CloudConnectWizard provider="dropbox" />;
}

export function ConnectGCSWizard() {
  return <ConnectGCSWizardInner />;
}
