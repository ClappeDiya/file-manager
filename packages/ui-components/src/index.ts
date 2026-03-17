/**
 * @ufop/ui-components
 *
 * Base UI component library for the Unified File Operations Platform.
 * Built with shadcn/ui patterns, Tailwind CSS, and React Aria for accessibility.
 */

// Utilities
export { cn } from "./utils";

// Components
export { Button, type ButtonProps } from "./button";
export { Input, type InputProps } from "./input";
export { Badge, type BadgeProps } from "./badge";
export { Separator, type SeparatorProps } from "./separator";
export { Tooltip, type TooltipProps } from "./tooltip";
export { ScrollArea, type ScrollAreaProps } from "./scroll-area";

// Dialog
export {
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  DialogClose,
  type DialogProps,
  type DialogContentProps,
} from "./dialog";

// Dropdown Menu
export {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuLabel,
  type DropdownMenuProps,
  type DropdownMenuContentProps,
  type DropdownMenuItemProps,
} from "./dropdown-menu";

// Tabs
export {
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
  type TabsProps,
  type TabsTriggerProps,
  type TabsContentProps,
} from "./tabs";
