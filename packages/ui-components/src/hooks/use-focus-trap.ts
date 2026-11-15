"use client";

import { useEffect, useRef } from "react";

const FOCUSABLE_SELECTOR =
  'a[href],button:not([disabled]),textarea:not([disabled]),input:not([disabled]),select:not([disabled]),[tabindex]:not([tabindex="-1"])';

const activeTraps: HTMLElement[] = [];
const inertOwners = new WeakMap<HTMLElement, { count: number; previous: boolean }>();

interface UseFocusTrapOptions {
  /** When false the trap is inactive (e.g. dialog is closed). */
  active: boolean;
  initialFocusSelector?: string;
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
  initialFocusSelector,
  onEscape,
  initialFocusRef,
}: UseFocusTrapOptions) {
  const containerRef = useRef<T | null>(null);

  const escapeRef = useRef(onEscape);
  useEffect(() => {
    escapeRef.current = onEscape;
  }, [onEscape]);

  useEffect(() => {
    if (!active) return;
    const container = containerRef.current;
    if (!container) return;
    const previouslyFocused = document.activeElement as HTMLElement | null;
    activeTraps.push(container);
    const isTop = () => activeTraps[activeTraps.length - 1] === container;
    const isolated: HTMLElement[] = [];
    let branch: HTMLElement | null = container;
    while (branch && branch !== document.body) {
      for (const sibling of Array.from(branch.parentElement?.children ?? [])) {
        if (sibling !== branch && sibling instanceof HTMLElement) {
          const state = inertOwners.get(sibling) ?? { count: 0, previous: sibling.inert };
          state.count++;
          inertOwners.set(sibling, state);
          sibling.inert = true;
          isolated.push(sibling);
        }
      }
      branch = branch.parentElement;
    }

    const focusFirst = () => {
      if (!isTop()) return;
      const target =
        initialFocusRef?.current ??
        (initialFocusSelector
          ? container?.querySelector<HTMLElement>(initialFocusSelector)
          : null) ??
        container?.querySelector<HTMLElement>(FOCUSABLE_SELECTOR);
      target?.focus();
    };

    // Defer one frame so portal-mounted children are present.
    const id = requestAnimationFrame(focusFirst);

    const onKeyDown = (event: KeyboardEvent) => {
      if (!isTop()) return;
      if (event.key === "Escape" && escapeRef.current) {
        event.preventDefault();
        escapeRef.current();
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
      if (!active || !container.contains(active)) {
        event.preventDefault();
        (event.shiftKey ? last : first).focus();
      } else if (event.shiftKey && active === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && active === last) {
        event.preventDefault();
        first.focus();
      }
    };

    const onFocus = (event: FocusEvent) => {
      if (isTop() && event.target instanceof Node && !container.contains(event.target)) focusFirst();
    };

    document.addEventListener("keydown", onKeyDown);
    document.addEventListener("focusin", onFocus);
    return () => {
      cancelAnimationFrame(id);
      document.removeEventListener("keydown", onKeyDown);
      document.removeEventListener("focusin", onFocus);
      const index = activeTraps.lastIndexOf(container);
      if (index >= 0) activeTraps.splice(index, 1);
      for (const sibling of isolated) {
        const state = inertOwners.get(sibling);
        if (state && --state.count === 0) {
          sibling.inert = state.previous;
          inertOwners.delete(sibling);
        }
      }
      if (previouslyFocused?.isConnected) previouslyFocused.focus();
    };
  }, [active, initialFocusRef, initialFocusSelector]);

  return containerRef;
}
