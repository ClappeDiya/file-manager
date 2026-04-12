import {
  FileText,
  ArrowRightLeft,
  RefreshCcw,
  Zap,
  HardDrive,
  Layers,
  Bot,
  ShieldCheck,
  Wrench,
  Plug,
  Settings,
  Activity,
  CheckCircle2,
  XCircle,
  Ban,
  MinusCircle,
} from "lucide-react";

export const ENGINE_ICONS: Record<string, typeof Activity> = {
  fs: FileText,
  transfer: ArrowRightLeft,
  sync: RefreshCcw,
  automation: Zap,
  mount: HardDrive,
  spaces: Layers,
  ai: Bot,
  vault: ShieldCheck,
  compat: Wrench,
  connector: Plug,
  system: Settings,
};

export const ENGINE_LABELS: Record<string, string> = {
  fs: "File",
  transfer: "Transfer",
  sync: "Sync",
  automation: "Automation",
  mount: "Mount",
  spaces: "Space",
  ai: "AI",
  vault: "Vault",
  compat: "Compat",
  connector: "Connector",
  system: "System",
};

export const STATUS_META: Record<
  string,
  { Icon: typeof Activity; className: string }
> = {
  ok: { Icon: CheckCircle2, className: "text-emerald-500" },
  failed: { Icon: XCircle, className: "text-red-500" },
  cancelled: { Icon: Ban, className: "text-amber-500" },
  skipped: {
    Icon: MinusCircle,
    className: "text-[color:var(--color-text-muted)]",
  },
};

export const ALL_ENGINE_KEYS = Object.keys(ENGINE_ICONS) as string[];
