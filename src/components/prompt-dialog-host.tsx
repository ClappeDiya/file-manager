import { useEffect, useRef, useState } from "react";
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
    if (req) requestAnimationFrame(() => inputRef.current?.select());
  }, [req]);

  if (confirmReq) {
    const finish = (ok: boolean) => {
      confirmReq.resolve(ok);
      setConfirmReq(null);
    };
    return (
      <div
        className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/50"
        role="dialog"
        aria-modal="true"
        onMouseDown={(e) => e.target === e.currentTarget && finish(false)}
      >
        <div
          className="w-[420px] rounded-lg border border-border bg-background p-4 shadow-xl"
          onKeyDown={(e) => e.key === "Escape" && finish(false)}
        >
          <p className="mb-3 whitespace-pre-wrap text-sm">{confirmReq.message}</p>
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
              autoFocus
              className={
                confirmReq.danger
                  ? "rounded bg-red-600 px-3 py-1 text-sm text-white hover:bg-red-500"
                  : "rounded bg-blue-600 px-3 py-1 text-sm text-white hover:bg-blue-500"
              }
              onClick={() => finish(true)}
            >
              {confirmReq.confirmLabel}
            </button>
          </div>
        </div>
      </div>
    );
  }

  if (!req) return null;

  const finish = (result: string | null) => {
    req.resolve(result);
    setReq(null);
  };

  return (
    <div
      className="fixed inset-0 z-[1000] flex items-center justify-center bg-black/50"
      role="dialog"
      aria-modal="true"
      onMouseDown={(e) => e.target === e.currentTarget && finish(null)}
    >
      <form
        className="w-[420px] rounded-lg border border-border bg-background p-4 shadow-xl"
        onSubmit={(e) => {
          e.preventDefault();
          finish(value);
        }}
        onKeyDown={(e) => e.key === "Escape" && finish(null)}
      >
        <label className="mb-2 block text-sm font-medium" htmlFor="prompt-dialog-input">
          {req.message}
        </label>
        <input
          id="prompt-dialog-input"
          ref={inputRef}
          autoFocus
          value={value}
          onChange={(e) => setValue(e.target.value)}
          className="w-full rounded border border-border bg-transparent px-2 py-1.5 text-sm outline-none focus:ring-1 focus:ring-blue-500"
        />
        <div className="mt-3 flex justify-end gap-2">
          <button type="button" className="rounded px-3 py-1 text-sm hover:bg-muted" onClick={() => finish(null)}>
            Cancel
          </button>
          <button type="submit" className="rounded bg-blue-600 px-3 py-1 text-sm text-white hover:bg-blue-500">
            OK
          </button>
        </div>
      </form>
    </div>
  );
}
