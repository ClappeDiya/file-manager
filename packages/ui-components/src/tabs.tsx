'use client';

import * as React from "react";
import { cn } from "./utils";

/* ===== Tabs Context ===== */
interface TabsContextValue {
  activeTab: string;
  onTabChange: (value: string) => void;
}

const TabsContext = React.createContext<TabsContextValue | null>(null);

function useTabsContext() {
  const ctx = React.useContext(TabsContext);
  if (!ctx) throw new Error("Tabs components must be used within Tabs");
  return ctx;
}

/* ===== Tabs Root ===== */
export interface TabsProps extends React.HTMLAttributes<HTMLDivElement> {
  value?: string;
  defaultValue?: string;
  onValueChange?: (value: string) => void;
}

export function Tabs({
  value: controlledValue,
  defaultValue = "",
  onValueChange,
  children,
  className,
  ...props
}: TabsProps) {
  const [uncontrolledValue, setUncontrolledValue] = React.useState(defaultValue);

  const isControlled = controlledValue !== undefined;
  const activeTab = isControlled ? controlledValue : uncontrolledValue;
  const onTabChange = isControlled
    ? onValueChange!
    : (v: string) => {
        setUncontrolledValue(v);
        onValueChange?.(v);
      };

  return (
    <TabsContext.Provider value={{ activeTab, onTabChange }}>
      <div className={cn("flex flex-col", className)} {...props}>
        {children}
      </div>
    </TabsContext.Provider>
  );
}

/* ===== Tab List ===== */
export function TabsList({
  className,
  children,
  ...props
}: React.HTMLAttributes<HTMLDivElement>) {
  const tabsListRef = React.useRef<HTMLDivElement>(null);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    const tabs = tabsListRef.current?.querySelectorAll<HTMLButtonElement>(
      '[role="tab"]:not([disabled])',
    );
    if (!tabs || tabs.length === 0) return;

    const currentIndex = Array.from(tabs).findIndex(
      (tab) => tab === document.activeElement,
    );

    let nextIndex: number;

    switch (e.key) {
      case "ArrowRight":
        e.preventDefault();
        nextIndex = currentIndex < tabs.length - 1 ? currentIndex + 1 : 0;
        tabs[nextIndex].focus();
        tabs[nextIndex].click();
        break;
      case "ArrowLeft":
        e.preventDefault();
        nextIndex = currentIndex > 0 ? currentIndex - 1 : tabs.length - 1;
        tabs[nextIndex].focus();
        tabs[nextIndex].click();
        break;
      case "Home":
        e.preventDefault();
        tabs[0].focus();
        tabs[0].click();
        break;
      case "End":
        e.preventDefault();
        tabs[tabs.length - 1].focus();
        tabs[tabs.length - 1].click();
        break;
    }
  };

  return (
    <div
      ref={tabsListRef}
      role="tablist"
      className={cn(
        "inline-flex items-center gap-1 rounded-[var(--radius-md)]",
        "bg-[var(--color-bg-secondary)] p-1",
        "border border-[var(--color-border)]",
        className,
      )}
      onKeyDown={handleKeyDown}
      {...props}
    >
      {children}
    </div>
  );
}

/* ===== Tab Trigger ===== */
export interface TabsTriggerProps
  extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  value: string;
}

export function TabsTrigger({
  className,
  value,
  children,
  disabled,
  ...props
}: TabsTriggerProps) {
  const { activeTab, onTabChange } = useTabsContext();
  const isActive = activeTab === value;

  return (
    <button
      role="tab"
      type="button"
      aria-selected={isActive}
      aria-controls={`tabpanel-${value}`}
      id={`tab-${value}`}
      tabIndex={isActive ? 0 : -1}
      disabled={disabled}
      className={cn(
        "inline-flex items-center justify-center rounded-[var(--radius-sm)] px-3 py-1.5",
        "text-[length:var(--font-size-sm)] font-medium whitespace-nowrap",
        "transition-theme cursor-pointer",
        "focus-visible:outline-2 focus-visible:outline-offset-1 focus-visible:outline-[var(--color-focus-ring)]",
        "disabled:pointer-events-none disabled:opacity-50",
        isActive
          ? "bg-[var(--color-bg)] text-[color:var(--color-text)] shadow-[var(--shadow-xs)]"
          : "text-[color:var(--color-text-secondary)] hover:text-[color:var(--color-text)] hover:bg-[var(--color-bg-tertiary)]",
        className,
      )}
      onClick={() => onTabChange(value)}
      {...props}
    >
      {children}
    </button>
  );
}

/* ===== Tab Content ===== */
export interface TabsContentProps extends React.HTMLAttributes<HTMLDivElement> {
  value: string;
}

export function TabsContent({
  className,
  value,
  children,
  ...props
}: TabsContentProps) {
  const { activeTab } = useTabsContext();

  if (activeTab !== value) return null;

  return (
    <div
      role="tabpanel"
      id={`tabpanel-${value}`}
      aria-labelledby={`tab-${value}`}
      tabIndex={0}
      className={cn("mt-2 animate-fade-in", className)}
      {...props}
    >
      {children}
    </div>
  );
}
