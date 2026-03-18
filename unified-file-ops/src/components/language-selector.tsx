/**
 * T-065: Language Selector Component
 *
 * Dropdown UI for switching application locale.
 * Displays languages in their native names and shows a checkmark next to the active locale.
 * Also provides a settings-panel variant for the Settings page.
 */

import { useEffect, useCallback, useState, useRef } from "react";
import { Globe } from "lucide-react";
import {
  useI18n,
  SUPPORTED_LOCALES,
  LOCALE_NATIVE_NAMES,
  type Locale,
} from "@/lib/i18n";
import { cn } from "@/lib/utils";

// ---------------------------------------------------------------------------
// Compact dropdown (for toolbar / header)
// ---------------------------------------------------------------------------

interface LanguageSelectorProps {
  /** Additional CSS classes for the trigger button */
  className?: string;
  /** Compact mode shows only the globe icon */
  compact?: boolean;
}

export function LanguageSelector({
  className,
  compact = false,
}: LanguageSelectorProps) {
  const { locale, setLocale, t, isLoading, init, isInitialized } = useI18n();
  const [isOpen, setIsOpen] = useState(false);
  const containerRef = useRef<HTMLDivElement>(null);

  // Initialize i18n on mount
  useEffect(() => {
    if (!isInitialized) {
      init();
    }
  }, [init, isInitialized]);

  // Close dropdown on outside click
  useEffect(() => {
    function handleClickOutside(event: MouseEvent) {
      if (
        containerRef.current &&
        !containerRef.current.contains(event.target as Node)
      ) {
        setIsOpen(false);
      }
    }

    if (isOpen) {
      document.addEventListener("mousedown", handleClickOutside);
      return () =>
        document.removeEventListener("mousedown", handleClickOutside);
    }
  }, [isOpen]);

  // Close on Escape
  useEffect(() => {
    function handleEscape(event: KeyboardEvent) {
      if (event.key === "Escape") {
        setIsOpen(false);
      }
    }

    if (isOpen) {
      document.addEventListener("keydown", handleEscape);
      return () => document.removeEventListener("keydown", handleEscape);
    }
  }, [isOpen]);

  const handleSelect = useCallback(
    async (newLocale: Locale) => {
      setIsOpen(false);
      if (newLocale !== locale) {
        await setLocale(newLocale);
      }
    },
    [locale, setLocale],
  );

  return (
    <div ref={containerRef} className={cn("relative inline-block", className)}>
      {/* Trigger button */}
      <button
        type="button"
        onClick={() => setIsOpen(!isOpen)}
        disabled={isLoading}
        className={cn(
          "inline-flex items-center gap-1.5 rounded-md px-2 py-1.5",
          "text-sm font-medium transition-colors",
          "hover:bg-accent hover:text-accent-foreground",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
          "disabled:pointer-events-none disabled:opacity-50",
        )}
        aria-label={t("settings.language")}
        aria-expanded={isOpen}
        aria-haspopup="listbox"
      >
        <Globe className="h-4 w-4" />
        {!compact && (
          <span className="hidden sm:inline">
            {LOCALE_NATIVE_NAMES[locale]}
          </span>
        )}
      </button>

      {/* Dropdown menu */}
      {isOpen && (
        <div
          role="listbox"
          aria-label={t("settings.language")}
          className={cn(
            "absolute right-0 z-50 mt-1 w-52 rounded-lg border bg-popover p-1 shadow-md",
            "animate-in fade-in-0 zoom-in-95",
          )}
        >
          {SUPPORTED_LOCALES.map((loc) => (
            <button
              key={loc}
              type="button"
              role="option"
              aria-selected={loc === locale}
              onClick={() => handleSelect(loc)}
              className={cn(
                "flex w-full items-center gap-2 rounded-md px-3 py-2 text-sm",
                "transition-colors hover:bg-accent hover:text-accent-foreground",
                "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                loc === locale && "bg-accent/50 font-medium",
              )}
            >
              <span className="flex-1 text-left">
                {LOCALE_NATIVE_NAMES[loc]}
              </span>
              {loc === locale && (
                <svg
                  className="h-4 w-4 shrink-0"
                  fill="none"
                  viewBox="0 0 24 24"
                  stroke="currentColor"
                  strokeWidth={2}
                  aria-hidden="true"
                >
                  <path
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    d="M5 13l4 4L19 7"
                  />
                </svg>
              )}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

// ---------------------------------------------------------------------------
// Settings panel variant (for the Settings page)
// ---------------------------------------------------------------------------

export function LanguageSettingsPanel() {
  const { locale, setLocale, t, isLoading, init, isInitialized } = useI18n();

  useEffect(() => {
    if (!isInitialized) {
      init();
    }
  }, [init, isInitialized]);

  const handleChange = useCallback(
    async (newLocale: Locale) => {
      if (newLocale !== locale) {
        await setLocale(newLocale);
      }
    },
    [locale, setLocale],
  );

  return (
    <div className="space-y-4">
      <div>
        <h3 className="text-sm font-medium">{t("settings.language")}</h3>
        <p className="text-sm text-muted-foreground">
          {t("settings.languageDescription")}
        </p>
      </div>

      <div className="grid grid-cols-2 gap-2 sm:grid-cols-4" role="radiogroup" aria-label={t("settings.language")}>
        {SUPPORTED_LOCALES.map((loc) => (
          <button
            key={loc}
            type="button"
            role="radio"
            aria-checked={loc === locale}
            disabled={isLoading}
            onClick={() => handleChange(loc)}
            className={cn(
              "flex flex-col items-center gap-1 rounded-lg border p-3",
              "text-sm transition-colors",
              "hover:bg-accent hover:text-accent-foreground",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
              "disabled:pointer-events-none disabled:opacity-50",
              loc === locale
                ? "border-primary bg-primary/5 font-medium"
                : "border-border",
            )}
          >
            <span className="text-base">{LOCALE_NATIVE_NAMES[loc]}</span>
            <span className="text-xs text-muted-foreground uppercase">
              {loc}
            </span>
          </button>
        ))}
      </div>
    </div>
  );
}
