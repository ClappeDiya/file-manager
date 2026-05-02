import { useEffect, useRef } from "react";

const FOCUSABLE_SELECTOR =
  'a[href],button:not([disabled]),textarea:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';

interface UseFocusTrapOptions {
  /** When false the trap is inactive (e.g. dialog is closed). */
  active: boolean;
  /** Optional callback invoked when the user presses Escape. */
  onEscape?: () => void;
  /** When provided, this element receives initial focus instead of the first focusable. */
  initialFocusRef?: React.RefObject<HTMLElement | null>;
}

/**
 * Focus trap for modal dialogs. While `active`:
 *  1. Initial focus moves to `initialFocusRef.current` or the first focusable.
 *  2. Tab / Shift+Tab cycle within the container instead of escaping.
 *  3. Escape calls `onEscape`.
 *  4. On unmount the previously-focused element regains focus.
 */
export function useFocusTrap<T extends HTMLElement>({
  active,
  onEscape,
  initialFocusRef,
}: UseFocusTrapOptions) {
  const containerRef = useRef<T | null>(null);

  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    const previouslyFocused = document.activeElement as HTMLElement | null;

    const focusFirst = () => {
      const target = initialFocusRef?.current
        ?? container?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      target?.focus();
    };

    // Defer one frame so portal-mounted children are present.
    const id = requestAnimationFrame(focusFirst);

    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key === "Escape" && onEscape) {
        event.preventDefault();
        onEscape();
        return;
      }
      if (event.key !== "Tab" || !container) return;
      const items = container.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR);
      if (items.length === 0) {
        event.preventDefault();
        return;
      }
      const first = items[0];
      const last = items[items.length - 1];
      const active = document.activeElement as HTMLElement | null;
      if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    document.addEventListener("keydown", onKeyDown);
    return () => {
      cancelAnimationFrame(id);
      document.removeEventListener("keydown", onKeyDown);
      previouslyFocused?.focus?.();
    };
  }, [active, onEscape, initialFocusRef]);

  return containerRef;
}
