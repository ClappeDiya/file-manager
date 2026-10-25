/**
 * Help > Send feedback: a message and a category, plus an optional (off by
 * default) scrubbed diagnostics bundle. With a management console configured
 * the feedback is sent there; without one it can only be saved to a file, and
 * the dialog says so. Nothing is called "sent" unless the console accepted it.
 */
import { useEffect, useState } from "react";
import { X } from "lucide-react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";
import { pickFolder } from "@/lib/folder-picker";
import { useFeedbackDialog } from "@/stores/feedback-store";

export type FeedbackCategory = "bug" | "idea" | "question" | "other";

export type FeedbackDestination = { type: "console"; console_url: string; organization: string } | { type: "file" };

export type FeedbackOutcome =
  | { type: "sent"; id: string; console_url: string; includes_diagnostics: boolean }
  | { type: "saved"; path: string; size: number; includes_diagnostics: boolean };

export const MAX_FEEDBACK_CHARS = 5000;

const CATEGORIES: { value: FeedbackCategory; label: string }[] = [
  { value: "bug", label: "Something is wrong" },
  { value: "idea", label: "An idea or request" },
  { value: "question", label: "A question" },
  { value: "other", label: "Something else" },
];

const field = "w-full text-sm px-2 py-1.5 rounded border border-[var(--color-border)] bg-[var(--color-bg)]";
const button = "text-sm px-3 py-1.5 rounded border border-[var(--color-border)] hover:bg-[var(--color-hover-bg)] disabled:opacity-50";

export function FeedbackForm({ onDone }: { onDone?: () => void }) {
  const [destination, setDestination] = useState<FeedbackDestination | null>(null);
  const [category, setCategory] = useState<FeedbackCategory>("bug");
  const [message, setMessage] = useState("");
  const [diagnostics, setDiagnostics] = useState(false);
  const [folder, setFolder] = useState("");
  // After a failed send the person can choose to save the feedback instead.
  const [saveInstead, setSaveInstead] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState<FeedbackOutcome | null>(null);

  useEffect(() => {
    let live = true;
    tauriInvoke<FeedbackDestination>("feedback_destination", undefined, { type: "file" })
      .then((d) => live && setDestination(d))
      .catch(() => live && setDestination({ type: "file" }));
    return () => {
      live = false;
    };
  }, []);

  const toConsole = destination?.type === "console" && !saveInstead;
  const needsFolder = !toConsole;
  const text = message.trim();
  const ready = destination !== null && text.length > 0 && text.length <= MAX_FEEDBACK_CHARS && (!needsFolder || folder.trim().length > 0);

  const submit = async () => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await tauriInvoke<FeedbackOutcome>("feedback_submit", {
        request: { category, message: text, include_diagnostics: diagnostics, save_path: needsFolder ? folder.trim() : null, save_only: saveInstead },
      });
      setDone(outcome);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  if (done) {
    return (
      <div className="space-y-3" data-testid="feedback-done">
        <p className="text-sm" role="status">
          {done.type === "sent" ? (
            <>Thank you. Your feedback was sent to the management console at {done.console_url} (reference {done.id}).</>
          ) : (
            <>
              Your feedback was saved to <span className="font-mono break-all">{done.path}</span>. It has not been sent anywhere: send that file to the people you are giving feedback to.
            </>
          )}
          {done.includes_diagnostics ? " It includes the diagnostics bundle." : " It does not include diagnostics."}
        </p>
        {onDone && (
          <button type="button" className={button} onClick={onDone}>
            Close
          </button>
        )}
      </div>
    );
  }

  return (
    <div className="space-y-3">
      <label className="block text-xs font-medium space-y-1">
        <span>What is it about?</span>
        <select className={field} value={category} onChange={(e) => setCategory(e.target.value as FeedbackCategory)} aria-label="Category">
          {CATEGORIES.map((c) => (
            <option key={c.value} value={c.value}>
              {c.label}
            </option>
          ))}
        </select>
      </label>

      <label className="block text-xs font-medium space-y-1">
        <span>Your message</span>
        <textarea className={`${field} h-32 resize-y`} value={message} maxLength={MAX_FEEDBACK_CHARS} onChange={(e) => setMessage(e.target.value)} aria-label="Message" placeholder="What happened, or what would you like?" />
        <span className="block text-[11px] font-normal text-[color:var(--color-text-secondary)]">
          {message.length}/{MAX_FEEDBACK_CHARS}. Your message is sent as you wrote it: leave out passwords and private details.
        </span>
      </label>

      <label className="flex items-start gap-2 text-sm">
        <input type="checkbox" className="mt-1" checked={diagnostics} onChange={(e) => setDiagnostics(e.target.checked)} aria-label="Include diagnostics" />
        <span>
          Include a diagnostics bundle
          <span className="block text-[11px] text-[color:var(--color-text-secondary)]">
            Recent logs, app version and OS details. Passwords, tokens, e-mail addresses and folder names are removed first. Without it, only your message, its category, the app version and your operating system are included.
          </span>
        </span>
      </label>

      {destination && (
        <div className="text-xs rounded border border-[var(--color-border)] p-2 space-y-2" data-testid="feedback-destination">
          {toConsole ? (
            <p>
              This will be sent to your management console{destination.type === "console" && destination.organization ? ` (${destination.organization})` : ""} at <span className="font-mono">{destination.type === "console" ? destination.console_url : ""}</span>.
            </p>
          ) : (
            <>
              <p>
                {destination.type === "console"
                  ? "It will be saved to a file instead of being sent."
                  : "This computer is not connected to a management console, so the feedback cannot be sent from here. It will be saved to a file you can pass on."}
              </p>
              <div className="flex gap-2">
                <input className={`${field} font-mono flex-1`} value={folder} onChange={(e) => setFolder(e.target.value)} placeholder="Folder to save it in" aria-label="Folder to save the feedback" />
                <button
                  type="button"
                  className={button}
                  onClick={async () => {
                    const picked = await pickFolder("Where to save the feedback");
                    if (picked) setFolder(picked);
                  }}
                >
                  Browse…
                </button>
              </div>
            </>
          )}
        </div>
      )}

      {error && (
        <div className="text-xs text-red-500 space-y-1" role="alert">
          <p>{error}</p>
          {destination?.type === "console" && !saveInstead && (
            <button type="button" className="underline" onClick={() => setSaveInstead(true)}>
              Save it to a file instead
            </button>
          )}
        </div>
      )}

      <div className="flex gap-2 justify-end">
        {onDone && (
          <button type="button" className={button} onClick={onDone} disabled={busy}>
            Cancel
          </button>
        )}
        <button type="button" className="text-sm px-3 py-1.5 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50" disabled={!ready || busy} onClick={() => void submit()}>
          {busy ? "Working…" : toConsole ? "Send feedback" : "Save feedback to file"}
        </button>
      </div>
    </div>
  );
}

/** The dialog around [`FeedbackForm`], opened by the command palette and Settings. */
export function FeedbackDialog() {
  const open = useFeedbackDialog((s) => s.open);
  const setOpen = useFeedbackDialog((s) => s.setOpen);
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-[60] flex items-center justify-center bg-black/50 backdrop-blur-sm" role="dialog" aria-modal="true" aria-label="Send feedback" data-testid="feedback-backdrop" onClick={() => setOpen(false)}>
      <div className="w-[min(560px,95vw)] max-h-[92vh] overflow-auto rounded-lg shadow-2xl border border-[var(--color-border)] bg-[var(--color-surface)] p-4 relative" onClick={(e) => e.stopPropagation()}>
        <div className="flex items-center mb-3">
          <h2 className="text-sm font-semibold">Send feedback</h2>
          <button className="ml-auto h-7 w-7 flex items-center justify-center rounded hover:bg-[var(--color-hover)]" onClick={() => setOpen(false)} aria-label="Close feedback">
            <X className="w-4 h-4" />
          </button>
        </div>
        {/* A fresh form each time it opens. */}
        <FeedbackForm onDone={() => setOpen(false)} />
      </div>
    </div>
  );
}
