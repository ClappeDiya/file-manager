import { useEffect, useRef, useState } from "react";
import { WorkspaceDialog } from "./workspace-dialog";
import {
  setPromptListener,
  setConfirmListener,
  type PromptRequest,
  type ConfirmRequest,
} from "@/lib/prompt-dialog";

export function PromptDialogHost() {
  const [req, setReq] = useState<PromptRequest | null>(null);
  const [confirmReq, setConfirmReq] = useState<ConfirmRequest | null>(null);
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement | null>(null);

  useEffect(() => {
    setPromptListener((r) => {
      setReq(r);
      setValue(r?.defaultValue ?? "");
    });
    setConfirmListener(setConfirmReq);
    return () => {
      setPromptListener(null);
      setConfirmListener(null);
    };
  }, []);

  useEffect(() => {
    if (!req) return;
    const id = requestAnimationFrame(() => inputRef.current?.select());
    return () => cancelAnimationFrame(id);
  }, [req]);

  if (confirmReq) {
    const finish = (ok: boolean) => {
      confirmReq.resolve(ok);
      setConfirmReq(null);
    };
    return (
      <WorkspaceDialog
        key="confirm"
        className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/50"
        aria-labelledby="confirm-dialog-message"
        onClose={() => finish(false)}
        closeOnBackdrop={false}
        onMouseDown={(e) => e.target === e.currentTarget && finish(false)}
      >
        <div className="w-[min(420px,90vw)] max-h-[90vh] overflow-y-auto rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] text-[color:var(--color-text)] p-4 shadow-xl">
          <p
            id="confirm-dialog-message"
            className="mb-3 whitespace-pre-wrap break-words text-sm"
          >
            {confirmReq.message}
          </p>
          <div className="flex justify-end gap-2">
            <button
              type="button"
              className="rounded px-3 py-1 text-sm hover:bg-muted"
              onClick={() => finish(false)}
            >
              Cancel
            </button>
            <button
              type="button"
              className={
                confirmReq.danger
                  ? "rounded bg-[var(--color-destructive)] px-3 py-1 text-sm text-[color:var(--color-destructive-foreground)] hover:bg-[var(--color-destructive-hover)]"
                  : "rounded bg-[var(--color-primary)] px-3 py-1 text-sm text-[color:var(--color-primary-foreground)] hover:bg-[var(--color-primary-hover)]"
              }
              onClick={() => finish(true)}
            >
              {confirmReq.confirmLabel}
            </button>
          </div>
        </div>
      </WorkspaceDialog>
    );
  }

  if (!req) return null;

  const finish = (result: string | null) => {
    req.resolve(result);
    setReq(null);
  };

  return (
    <WorkspaceDialog
      key="prompt"
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/50"
      aria-label="Enter a value"
      aria-describedby="prompt-dialog-label"
      onClose={() => finish(null)}
      closeOnBackdrop={false}
      onMouseDown={(e) => e.target === e.currentTarget && finish(null)}
    >
      <form
        className="w-[min(420px,90vw)] max-h-[90vh] overflow-y-auto rounded-lg border border-[var(--color-border)] bg-[var(--color-surface)] text-[color:var(--color-text)] p-4 shadow-xl"
        onSubmit={(e) => {
          e.preventDefault();
          finish(value);
        }}
      >
        <label
          id="prompt-dialog-label"
          className="mb-2 block break-words text-sm font-medium"
          htmlFor="prompt-dialog-input"
        >
          {req.message}
        </label>
        <input
          id="prompt-dialog-input"
          ref={inputRef}
          value={value}
          onChange={(e) => setValue(e.target.value)}
          className="w-full rounded border border-border bg-transparent px-2 py-1.5 text-sm outline-none focus:ring-1 focus:ring-blue-500"
        />
        <div className="mt-3 flex justify-end gap-2">
          <button
            type="button"
            className="rounded px-3 py-1 text-sm hover:bg-muted"
            onClick={() => finish(null)}
          >
            Cancel
          </button>
          <button
            type="submit"
            className="rounded bg-[var(--color-primary)] px-3 py-1 text-sm text-[color:var(--color-primary-foreground)] hover:bg-[var(--color-primary-hover)]"
          >
            OK
          </button>
        </div>
      </form>
    </WorkspaceDialog>
  );
}
