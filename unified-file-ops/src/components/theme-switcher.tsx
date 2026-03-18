import { useUIStore, type Theme } from "@/stores/ui-store";
import { Sun, Moon, Monitor, Contrast } from "lucide-react";

const themes: { value: Theme; label: string; icon: React.ReactNode }[] = [
  { value: "light", label: "Light", icon: <Sun className="h-4 w-4" aria-hidden="true" /> },
  { value: "dark", label: "Dark", icon: <Moon className="h-4 w-4" aria-hidden="true" /> },
  { value: "system", label: "System", icon: <Monitor className="h-4 w-4" aria-hidden="true" /> },
  {
    value: "high-contrast",
    label: "High Contrast",
    icon: <Contrast className="h-4 w-4" aria-hidden="true" />,
  },
];

/**
 * Theme switcher component with radio-group semantics.
 * Provides instant theme switching with keyboard accessibility.
 */
export function ThemeSwitcher() {
  const theme = useUIStore((s) => s.theme);
  const setTheme = useUIStore((s) => s.setTheme);

  return (
    <div
      role="radiogroup"
      aria-label="Theme selection"
      className="flex items-center gap-1 rounded-[var(--radius-md)] bg-[var(--color-bg-secondary)] p-1 border border-[var(--color-border)]"
    >
      {themes.map(({ value, label, icon }) => {
        const isSelected = theme === value;
        return (
          <button
            key={value}
            role="radio"
            aria-checked={isSelected}
            aria-label={`${label} theme`}
            onClick={() => setTheme(value)}
            className={`
              inline-flex items-center justify-center gap-1.5 rounded-[var(--radius-sm)] px-2.5 py-1.5
              text-[length:var(--font-size-sm)] font-medium whitespace-nowrap cursor-pointer
              transition-theme
              focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--color-focus-ring)]
              ${
                isSelected
                  ? "bg-[var(--color-bg)] text-[color:var(--color-text)] shadow-[var(--shadow-xs)]"
                  : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-bg-tertiary)]"
              }
            `}
            type="button"
          >
            {icon}
            <span className="hidden sm:inline">{label}</span>
          </button>
        );
      })}
    </div>
  );
}
