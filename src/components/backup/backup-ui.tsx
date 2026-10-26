/**
 * Small shared primitives for the Backup panel (DRY form layout, buttons,
 * folder picker). Styled to match the Sync panel.
 */
import { useState, type ReactNode } from "react";
import { inputCls } from "@/lib/backup-helpers";
import { pickFolder } from "@/lib/folder-picker";
import { RemoteFolderPicker } from "@/components/remote-folder-picker";

export function Section({ title, children, hint }: { title: string; children: ReactNode; hint?: string }) {
  return (
    <section className="border border-zinc-200 dark:border-zinc-700 rounded-lg p-3 space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-zinc-500 dark:text-zinc-400">{title}</h3>
      {hint && <p className="text-xs text-zinc-500 dark:text-zinc-400">{hint}</p>}
      {children}
    </section>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block text-xs text-zinc-700 dark:text-zinc-300 space-y-1">
      <span className="font-medium">{label}</span>
      {children}
      {hint && <span className="block text-[11px] text-zinc-500">{hint}</span>}
    </label>
  );
}


type Tone = "primary" | "neutral" | "danger" | "success";
const toneCls: Record<Tone, string> = {
  primary: "bg-blue-500 text-white hover:bg-blue-600",
  success: "bg-green-500 text-white hover:bg-green-600",
  danger: "bg-red-500 text-white hover:bg-red-600",
  neutral: "bg-zinc-100 dark:bg-zinc-800 text-zinc-700 dark:text-zinc-300 hover:bg-zinc-200 dark:hover:bg-zinc-700",
};

export function Btn({
  children,
  onClick,
  tone = "neutral",
  disabled,
  title,
  testId,
  type = "button",
}: {
  children: ReactNode;
  onClick?: () => void;
  tone?: Tone;
  disabled?: boolean;
  title?: string;
  testId?: string;
  type?: "button" | "submit";
}) {
  return (
    <button
      type={type}
      className={`px-2 py-1 text-xs rounded disabled:opacity-50 disabled:cursor-not-allowed ${toneCls[tone]}`}
      onClick={onClick}
      disabled={disabled}
      title={title}
      data-testid={testId}
    >
      {children}
    </button>
  );
}

export function Check({ label, checked, onChange, hint }: { label: string; checked: boolean; onChange: (v: boolean) => void; hint?: string }) {
  return (
    <label className="flex items-start gap-2 text-xs text-zinc-700 dark:text-zinc-300">
      <input type="checkbox" className="mt-0.5" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        {label}
        {hint && <span className="block text-[11px] text-zinc-500">{hint}</span>}
      </span>
    </label>
  );
}

export function Banner({ tone, children, onClose }: { tone: "error" | "info" | "success" | "warning"; children: ReactNode; onClose?: () => void }) {
  const cls = {
    error: "text-red-700 bg-red-50 dark:bg-red-900/20 dark:text-red-300 border-red-200 dark:border-red-800",
    info: "text-blue-700 bg-blue-50 dark:bg-blue-900/20 dark:text-blue-300 border-blue-200 dark:border-blue-800",
    success: "text-green-700 bg-green-50 dark:bg-green-900/20 dark:text-green-300 border-green-200 dark:border-green-800",
    warning: "text-amber-700 bg-amber-50 dark:bg-amber-900/20 dark:text-amber-300 border-amber-200 dark:border-amber-800",
  }[tone];
  return (
    <div className={`p-2 text-xs border rounded flex items-start justify-between gap-2 ${cls}`} role={tone === "error" ? "alert" : "status"}>
      <div className="min-w-0 break-words">{children}</div>
      {onClose && (
        <button className="underline shrink-0" onClick={onClose}>
          Dismiss
        </button>
      )}
    </div>
  );
}


/** Text input with a "Browse…" folder picker button (native) and
 *  "Pick…" (the in-app picker, which can also create the folder). */
export function PathInput({ value, onChange, placeholder, testId }: { value: string; onChange: (v: string) => void; placeholder?: string; testId?: string }) {
  const [picking, setPicking] = useState(false);
  return (
    <div className="flex gap-1">
      <input className={inputCls} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} data-testid={testId} />
      <Btn
        onClick={async () => {
          const p = await pickFolder("Choose folder");
          if (p) onChange(p);
        }}
      >
        Browse…
      </Btn>
      <Btn onClick={() => setPicking(true)} testId={testId ? `${testId}-pick` : undefined} title="Pick a folder, or create one">
        Pick…
      </Btn>
      {picking && (
        <RemoteFolderPicker
          title="Choose folder"
          initialPath={value || null}
          allowConnections={false}
          onCancel={() => setPicking(false)}
          onPick={(p) => {
            setPicking(false);
            onChange(p);
          }}
        />
      )}
    </div>
  );
}


