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
    <section className="border border-[var(--color-border)] rounded-lg p-3 space-y-2">
      <h3 className="text-xs font-semibold uppercase tracking-wide text-[color:var(--color-text-secondary)]">{title}</h3>
      {hint && <p className="text-xs text-[color:var(--color-text-secondary)]">{hint}</p>}
      {children}
    </section>
  );
}

export function Field({ label, children, hint }: { label: string; children: ReactNode; hint?: string }) {
  return (
    <label className="block text-xs text-[color:var(--color-text)] space-y-1">
      <span className="font-medium">{label}</span>
      {children}
      {hint && <span className="block text-[11px] text-[color:var(--color-text-secondary)]">{hint}</span>}
    </label>
  );
}


type Tone = "primary" | "neutral" | "danger" | "success";
const toneCls: Record<Tone, string> = {
  primary: "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:bg-[var(--color-primary-hover)]",
  success: "bg-[var(--color-primary)] text-[color:var(--color-primary-foreground)] hover:bg-[var(--color-primary-hover)]",
  danger: "bg-[var(--color-destructive)] text-[color:var(--color-destructive-foreground)] hover:bg-[var(--color-destructive-hover)]",
  neutral: "bg-[var(--color-bg-secondary)] text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
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
      className={`px-2 py-1 text-xs rounded focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)] disabled:opacity-50 disabled:cursor-not-allowed ${toneCls[tone]}`}
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
    <label className="flex items-start gap-2 text-xs text-[color:var(--color-text)]">
      <input type="checkbox" className="mt-0.5" checked={checked} onChange={(e) => onChange(e.target.checked)} />
      <span>
        {label}
        {hint && <span className="block text-[11px] text-[color:var(--color-text-secondary)]">{hint}</span>}
      </span>
    </label>
  );
}

export function Banner({ tone, children, onClose }: { tone: "error" | "info" | "success" | "warning"; children: ReactNode; onClose?: () => void }) {
  const cls = {
    error: "text-[color:var(--color-error-foreground)] bg-[var(--color-error-bg)] border-[var(--color-border)]",
    info: "text-[color:var(--color-info-foreground)] bg-[var(--color-info-bg)] border-[var(--color-border)]",
    success: "text-[color:var(--color-success-foreground)] bg-[var(--color-success-bg)] border-[var(--color-border)]",
    warning: "text-[color:var(--color-warning-foreground)] bg-[var(--color-warning-bg)] border-[var(--color-border)]",
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
export function PathInput({ value, onChange, placeholder, testId, ariaLabel }: { value: string; onChange: (v: string) => void; placeholder?: string; testId?: string; ariaLabel?: string }) {
  const [picking, setPicking] = useState(false);
  return (
    <div className="flex gap-1">
      <input className={inputCls} aria-label={ariaLabel} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} data-testid={testId} />
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


