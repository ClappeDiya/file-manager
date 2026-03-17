import { useRef } from "react";
import { useListBox, useOption, useFocusRing } from "react-aria";
import { useListState, type ListProps, Item } from "react-stately";
import { cn } from "@ufop/ui-components";
import type { Node } from "@react-types/shared";

/**
 * Accessible list component using React Aria hooks.
 * Supports full keyboard navigation (Arrow keys, Home, End, type-ahead).
 * All items have proper ARIA roles and screen reader labels.
 */

interface AccessibleListProps<T extends object> extends ListProps<T> {
  className?: string;
  label: string;
}

export function AccessibleList<T extends object>(
  props: AccessibleListProps<T>,
) {
  const state = useListState(props);
  const listRef = useRef<HTMLUListElement>(null);
  const { listBoxProps, labelProps } = useListBox(
    {
      ...props,
      "aria-label": props.label,
    },
    state,
    listRef,
  );

  return (
    <div className={cn("space-y-1", props.className)}>
      <span
        {...labelProps}
        className="text-[length:var(--font-size-sm)] font-medium text-[color:var(--color-text-secondary)]"
      >
        {props.label}
      </span>
      <ul
        {...listBoxProps}
        ref={listRef}
        className={cn(
          "rounded-[var(--radius-md)] border border-[var(--color-border)]",
          "bg-[var(--color-bg)] overflow-auto",
          "focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        style={{ maxHeight: "300px" }}
      >
        {[...state.collection].map((item) => (
          <AccessibleOption key={item.key} item={item} state={state} />
        ))}
      </ul>
    </div>
  );
}

interface OptionProps<T extends object> {
  item: Node<T>;
  state: ReturnType<typeof useListState<T>>;
}

function AccessibleOption<T extends object>({ item, state }: OptionProps<T>) {
  const ref = useRef<HTMLLIElement>(null);
  const { optionProps, isSelected, isDisabled } = useOption(
    { key: item.key },
    state,
    ref,
  );
  const { isFocusVisible, focusProps } = useFocusRing();

  return (
    <li
      {...optionProps}
      {...focusProps}
      ref={ref}
      className={cn(
        "flex items-center px-3 cursor-pointer select-none",
        "text-[length:var(--font-size-sm)]",
        "border-b border-[var(--color-border)] last:border-b-0",
        "transition-theme",
        isSelected
          ? "bg-[var(--color-selection-bg)] text-[color:var(--color-selection-text)]"
          : "text-[color:var(--color-text)] hover:bg-[var(--color-hover-bg)]",
        isDisabled && "opacity-50 pointer-events-none",
        isFocusVisible &&
          "outline-2 outline-offset-[-2px] outline-[var(--color-focus-ring)]",
      )}
      style={{ height: "var(--file-row-height)" }}
    >
      {item.rendered}
    </li>
  );
}

// Re-export Item for convenience
export { Item as ListItem };
