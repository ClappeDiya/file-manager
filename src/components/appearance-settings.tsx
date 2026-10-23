/**
 * Settings → Appearance: theme, text size (K121) and display units. All
 * Standard settings, stored in the persisted UI store.
 */
import { FONT_SCALES, useUIStore, type ByteUnits, type FontScale, type TimeDisplay } from "@/stores/ui-store";
import { formatBytes } from "@/lib/format-bytes";
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

const UNIT_CHOICES: readonly { value: ByteUnits; label: string; hint: string }[] = [
  { value: "decimal", label: "Decimal (KB, MB, GB)", hint: "1 KB = 1,000 bytes, like macOS and disk makers" },
  { value: "binary", label: "Binary (KiB, MiB, GiB)", hint: "1 KiB = 1,024 bytes, like Windows Explorer's numbers" },
];
const TIME_CHOICES: readonly { value: TimeDisplay; label: string }[] = [
  { value: "local", label: "This computer's time" },
  { value: "utc", label: "UTC" },
];

const isMac = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

export function AppearanceSettings() {
  const fontScale = useUIStore((s) => s.fontScale);
  const setFontScale = useUIStore((s) => s.setFontScale);
  const byteUnits = useUIStore((s) => s.byteUnits);
  const setByteUnits = useUIStore((s) => s.setByteUnits);
  const timeDisplay = useUIStore((s) => s.timeDisplay);
  const setTimeDisplay = useUIStore((s) => s.setTimeDisplay);
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
      <Row label="Size units" hint={`Example: 1,500,000 bytes = ${formatBytes(1_500_000, { units: byteUnits })}. Used everywhere the app shows a size.`}>
        <ChoiceGroup<ByteUnits> label="Size units" value={byteUnits} options={UNIT_CHOICES} onChange={setByteUnits} testId="byte-units" />
      </Row>
      <Row label="Backup times" hint="How the Backup screens show the times of versions, runs and files.">
        <ChoiceGroup<TimeDisplay> label="Backup times" value={timeDisplay} options={TIME_CHOICES} onChange={setTimeDisplay} testId="time-display" />
      </Row>
    </div>
  );
}
