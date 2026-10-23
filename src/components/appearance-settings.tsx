/**
 * Settings → Appearance: theme, text size (K121) and display units. All
 * Standard settings, stored in the persisted UI store.
 */
import { FONT_SCALES, useUIStore, type FontScale } from "@/stores/ui-store";
import { ThemeSwitcher } from "./theme-switcher";

/** A labelled radio group of segmented buttons. */
export function ChoiceGroup<T extends string | number>({
  label,
  value,
  options,
  onChange,
  testId,
}: {
  label: string;
  value: T;
  options: readonly { value: T; label: string; hint?: string }[];
  onChange: (v: T) => void;
  testId?: string;
}) {
  return (
    <div role="radiogroup" aria-label={label} className="flex flex-wrap gap-1" data-testid={testId}>
      {options.map((o) => (
        <button
          key={String(o.value)}
          type="button"
          role="radio"
          aria-checked={o.value === value}
          title={o.hint}
          onClick={() => onChange(o.value)}
          className={`px-2.5 py-1 rounded border text-xs ${
            o.value === value
              ? "bg-blue-600 text-white border-blue-600"
              : "border-gray-300 dark:border-gray-600 hover:bg-gray-50 dark:hover:bg-gray-800"
          }`}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

function Row({ label, hint, children }: { label: string; hint?: string; children: React.ReactNode }) {
  return (
    <div className="space-y-1">
      <div className="text-xs font-medium">{label}</div>
      {children}
      {hint && <p className="text-[11px] text-gray-500">{hint}</p>}
    </div>
  );
}

const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

export function AppearanceSettings() {
  const fontScale = useUIStore((s) => s.fontScale);
  const setFontScale = useUIStore((s) => s.setFontScale);
  const mod = isMac ? "⌘" : "Ctrl";
  return (
    <div className="space-y-3 pt-3" data-testid="appearance-settings">
      <Row label="Theme">
        <ThemeSwitcher />
      </Row>
      <Row label="Text size" hint={`Shortcuts: ${mod} + (larger), ${mod} − (smaller), ${mod} 0 (default).`}>
        <ChoiceGroup<FontScale>
          label="Text size"
          value={fontScale}
          options={FONT_SCALES.map((f) => ({ value: f.value, label: `${f.label} (${Math.round(f.value * 100)}%)` }))}
          onChange={setFontScale}
          testId="text-size"
        />
      </Row>
    </div>
  );
}
