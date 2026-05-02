'use client';

import * as React from "react";
import { cn } from "./utils";

export interface ScrollAreaProps extends React.HTMLAttributes<HTMLDivElement> {
  /** Orientation for the scroll area */
  orientation?: "vertical" | "horizontal" | "both";
  /** Maximum height before scrolling */
  maxHeight?: string | number;
}

/**
 * A styled scroll area with custom scrollbar appearance.
 * Uses native scrolling for performance with styled scrollbar via CSS.
 */
export const ScrollArea = React.forwardRef<HTMLDivElement, ScrollAreaProps>(
  (
    { className, orientation = "vertical", maxHeight, style, children, ...props },
    ref,
  ) => {
    const overflowClass = {
      vertical: "overflow-y-auto overflow-x-hidden",
      horizontal: "overflow-x-auto overflow-y-hidden",
      both: "overflow-auto",
    };

    return (
      <div
        ref={ref}
        className={cn(
          "relative",
          overflowClass[orientation],
          // Custom scrollbar styles are defined in globals.css
          className,
        )}
        style={{
          maxHeight:
            typeof maxHeight === "number" ? `${maxHeight}px` : maxHeight,
          ...style,
        }}
        tabIndex={0}
        role="region"
        aria-label={props["aria-label"] || "Scrollable content"}
        {...props}
      >
        {children}
      </div>
    );
  },
);

ScrollArea.displayName = "ScrollArea";
