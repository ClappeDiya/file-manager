import { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  DialogDescription,
} from "@ufop/ui-components";
import { cn } from "@ufop/ui-components";
import {
  Folder,
  FolderHeart,
  Layers,
  Globe,
  Database,
  Briefcase,
  ChevronRight,
  ChevronLeft,
  X,
} from "lucide-react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { useSpacesStore, type SmartSpace } from "@/stores/spaces-store";
import { pickFolder } from "@/lib/folder-picker";

interface SmartSpaceWizardProps {
  onClose: () => void;
}

// ── Icon & Color Presets ──

const ICON_PRESETS = [
  { name: "folder", icon: Folder },
  { name: "folder-heart", icon: FolderHeart },
  { name: "layers", icon: Layers },
  { name: "globe", icon: Globe },
  { name: "database", icon: Database },
  { name: "briefcase", icon: Briefcase },
];

const COLOR_PRESETS = [
  "#6366f1", // indigo
  "#8b5cf6", // violet
  "#06b6d4", // cyan
  "#10b981", // emerald
  "#f59e0b", // amber
  "#ef4444", // red
  "#ec4899", // pink
  "#64748b", // slate
];

// ── Connection type for dropdown ──

interface ConnectionOption {
  id: string;
  name: string;
  protocol: string;
  remote_path: string;
}

// ── Sync pair type for dropdown ──

interface SyncPairOption {
  id: string;
  name: string;
  mode: string;
}

export function SmartSpaceWizard({ onClose }: SmartSpaceWizardProps) {
  const prefillLocalPath = useSpacesStore((s) => s.prefillLocalPath);
  const createSpace = useSpacesStore((s) => s.createSpace);

  const [step, setStep] = useState(1);

  // Step 1 — Identity
  const [name, setName] = useState("");
  const [icon, setIcon] = useState("folder-heart");
  const [color, setColor] = useState(COLOR_PRESETS[0]);
  const [localPath, setLocalPath] = useState(prefillLocalPath ?? "");

  // Step 2 — Remote (optional)
  const [connections, setConnections] = useState<ConnectionOption[]>([]);
  const [selectedConnectionId, setSelectedConnectionId] = useState<
    string | null
  >(null);
  const [remotePath, setRemotePath] = useState("");

  // Step 3 — Sync (optional)
  const [syncPairs, setSyncPairs] = useState<SyncPairOption[]>([]);
  const [selectedSyncPairId, setSelectedSyncPairId] = useState<string | null>(
    null,
  );

  const [creating, setCreating] = useState(false);

  // Load connections when entering step 2
  useEffect(() => {
    if (step === 2) {
      tauriInvoke<ConnectionOption[]>(
        "list_connections",
        undefined,
        [],
      ).then((conns) => {
        setConnections(conns);
      });
    }
  }, [step]);

  // Load sync pairs when entering step 3
  useEffect(() => {
    if (step === 3) {
      tauriInvoke<SyncPairOption[]>(
        "list_sync_pairs",
        undefined,
        [],
      ).then((pairs) => {
        setSyncPairs(pairs);
      });
    }
  }, [step]);

  const handlePickFolder = async () => {
    const path = await pickFolder("Select local folder for Smart Space");
    if (path) {
      setLocalPath(path);
      if (!name) {
        setName(path.split("/").filter(Boolean).pop() ?? "");
      }
    }
  };

  const canProceedStep1 = name.trim().length > 0 && localPath.trim().length > 0;

  const handleCreate = async () => {
    setCreating(true);
    const now = new Date().toISOString();
    const space: SmartSpace = {
      id: crypto.randomUUID(),
      name: name.trim(),
      icon,
      color,
      local_path: localPath.trim(),
      connection_id: selectedConnectionId,
      remote_path: remotePath.trim() || null,
      sync_pair_id: selectedSyncPairId,
      enabled: true,
      created_at: now,
      updated_at: now,
    };

    await createSpace(space);
    setCreating(false);
    onClose();
  };

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <div className="flex items-start justify-between">
            <div className="flex-1 min-w-0">
              <DialogTitle>
                {step === 1 && "Create Smart Space"}
                {step === 2 && "Connect Remote (Optional)"}
                {step === 3 && "Sync Rule (Optional)"}
              </DialogTitle>
              <DialogDescription>
                {step === 1 && "Bundle a local folder with a remote connection."}
                {step === 2 && "Link an existing connection or skip for local-only."}
                {step === 3 && "Attach a sync pair or skip."}
              </DialogDescription>
            </div>
            <button
              type="button"
              onClick={onClose}
              className={cn(
                "shrink-0 ml-2 p-1 rounded-md",
                "text-[color:var(--color-text-tertiary)]",
                "hover:text-[color:var(--color-text-primary)] hover:bg-[var(--color-hover-bg)]",
                "transition-colors",
              )}
              aria-label="Close"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
        </DialogHeader>

        <div className="space-y-4 py-2">
          {/* ── Step 1: Identity ── */}
          {step === 1 && (
            <>
              {/* Name */}
              <div>
                <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                  Name
                </label>
                <input
                  type="text"
                  className={cn(
                    "w-full rounded-md border px-3 py-1.5 text-sm",
                    "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                    "text-[color:var(--color-text-primary)]",
                    "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
                  )}
                  placeholder="e.g., Client Assets"
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  autoFocus
                />
              </div>

              {/* Local folder */}
              <div>
                <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                  Local Folder
                </label>
                <div className="flex gap-2">
                  <input
                    type="text"
                    className={cn(
                      "flex-1 rounded-md border px-3 py-1.5 text-sm",
                      "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                      "text-[color:var(--color-text-primary)]",
                      "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
                    )}
                    placeholder="/path/to/folder"
                    value={localPath}
                    onChange={(e) => setLocalPath(e.target.value)}
                  />
                  <button
                    className={cn(
                      "rounded-md border px-3 py-1.5 text-sm shrink-0",
                      "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                      "text-[color:var(--color-text-secondary)]",
                      "hover:bg-[var(--color-hover-bg)]",
                    )}
                    onClick={handlePickFolder}
                    type="button"
                  >
                    Browse
                  </button>
                </div>
              </div>

              {/* Icon picker */}
              <div>
                <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                  Icon
                </label>
                <div className="flex gap-1.5">
                  {ICON_PRESETS.map((preset) => {
                    const Icon = preset.icon;
                    return (
                      <button
                        key={preset.name}
                        className={cn(
                          "p-1.5 rounded-md border",
                          icon === preset.name
                            ? "border-[var(--color-focus-ring)] bg-[var(--color-active-bg)]"
                            : "border-transparent hover:bg-[var(--color-hover-bg)]",
                        )}
                        onClick={() => setIcon(preset.name)}
                        type="button"
                        title={preset.name}
                      >
                        <Icon className="h-4 w-4 text-[color:var(--color-text-primary)]" />
                      </button>
                    );
                  })}
                </div>
              </div>

              {/* Color picker */}
              <div>
                <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                  Color
                </label>
                <div className="flex gap-1.5">
                  {COLOR_PRESETS.map((c) => (
                    <button
                      key={c}
                      className={cn(
                        "h-6 w-6 rounded-full border-2",
                        color === c
                          ? "border-[color:var(--color-text-primary)] scale-110"
                          : "border-transparent",
                      )}
                      style={{ backgroundColor: c }}
                      onClick={() => setColor(c)}
                      type="button"
                      title={c}
                    />
                  ))}
                </div>
              </div>
            </>
          )}

          {/* ── Step 2: Remote ── */}
          {step === 2 && (
            <>
              {connections.length === 0 ? (
                <p className="text-sm text-[color:var(--color-text-tertiary)] py-4">
                  No connections configured yet. You can add connections in the
                  Connections panel and link them later.
                </p>
              ) : (
                <>
                  <div>
                    <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                      Connection
                    </label>
                    <select
                      className={cn(
                        "w-full rounded-md border px-3 py-1.5 text-sm",
                        "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                        "text-[color:var(--color-text-primary)]",
                        "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
                      )}
                      value={selectedConnectionId ?? ""}
                      onChange={(e) => {
                        const id = e.target.value || null;
                        setSelectedConnectionId(id);
                        if (id) {
                          const conn = connections.find((c) => c.id === id);
                          if (conn?.remote_path) setRemotePath(conn.remote_path);
                        }
                      }}
                    >
                      <option value="">None (local only)</option>
                      {connections.map((conn) => (
                        <option key={conn.id} value={conn.id}>
                          {conn.name} ({conn.protocol})
                        </option>
                      ))}
                    </select>
                  </div>

                  {selectedConnectionId && (
                    <div>
                      <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                        Remote Path
                      </label>
                      <input
                        type="text"
                        className={cn(
                          "w-full rounded-md border px-3 py-1.5 text-sm",
                          "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                          "text-[color:var(--color-text-primary)]",
                          "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
                        )}
                        placeholder="/"
                        value={remotePath}
                        onChange={(e) => setRemotePath(e.target.value)}
                      />
                    </div>
                  )}
                </>
              )}
            </>
          )}

          {/* ── Step 3: Sync ── */}
          {step === 3 && (
            <>
              {syncPairs.length === 0 ? (
                <p className="text-sm text-[color:var(--color-text-tertiary)] py-4">
                  No sync pairs configured yet. You can create sync pairs in the
                  Sync panel and link them later.
                </p>
              ) : (
                <div>
                  <label className="block text-[length:var(--font-size-xs)] font-medium text-[color:var(--color-text-secondary)] mb-1">
                    Sync Pair
                  </label>
                  <select
                    className={cn(
                      "w-full rounded-md border px-3 py-1.5 text-sm",
                      "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                      "text-[color:var(--color-text-primary)]",
                      "focus:outline-none focus:ring-2 focus:ring-[var(--color-focus-ring)]",
                    )}
                    value={selectedSyncPairId ?? ""}
                    onChange={(e) =>
                      setSelectedSyncPairId(e.target.value || null)
                    }
                  >
                    <option value="">None (no sync)</option>
                    {syncPairs.map((pair) => (
                      <option key={pair.id} value={pair.id}>
                        {pair.name} ({pair.mode})
                      </option>
                    ))}
                  </select>
                </div>
              )}
            </>
          )}
        </div>

        {/* ── Footer / Navigation ── */}
        <DialogFooter>
          <div className="flex items-center justify-between w-full">
            {/* Step indicator */}
            <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Step {step} of 3
            </span>

            <div className="flex gap-2">
              <button
                className={cn(
                  "rounded-md border px-3 py-1.5 text-sm",
                  "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                  "text-[color:var(--color-text-tertiary)]",
                  "hover:bg-[var(--color-hover-bg)]",
                )}
                onClick={onClose}
                type="button"
              >
                Cancel
              </button>
              {step > 1 && (
                <button
                  className={cn(
                    "flex items-center gap-1 rounded-md border px-3 py-1.5 text-sm",
                    "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                    "text-[color:var(--color-text-secondary)]",
                    "hover:bg-[var(--color-hover-bg)]",
                  )}
                  onClick={() => setStep(step - 1)}
                  type="button"
                >
                  <ChevronLeft className="h-3.5 w-3.5" />
                  Back
                </button>
              )}

              {step < 3 ? (
                <>
                  {step === 2 && (
                    <button
                      className={cn(
                        "rounded-md border px-3 py-1.5 text-sm",
                        "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                        "text-[color:var(--color-text-tertiary)]",
                        "hover:bg-[var(--color-hover-bg)]",
                      )}
                      onClick={() => {
                        setSelectedConnectionId(null);
                        setRemotePath("");
                        setStep(3);
                      }}
                      type="button"
                    >
                      Skip
                    </button>
                  )}
                  <button
                    className={cn(
                      "flex items-center gap-1 rounded-md px-3 py-1.5 text-sm font-medium",
                      "bg-[var(--color-accent)] text-white",
                      "hover:opacity-90",
                      "disabled:opacity-50 disabled:cursor-not-allowed",
                    )}
                    onClick={() => setStep(step + 1)}
                    disabled={step === 1 && !canProceedStep1}
                    type="button"
                  >
                    Next
                    <ChevronRight className="h-3.5 w-3.5" />
                  </button>
                </>
              ) : (
                <>
                  <button
                    className={cn(
                      "rounded-md border px-3 py-1.5 text-sm",
                      "bg-[var(--color-input-bg)] border-[var(--color-border)]",
                      "text-[color:var(--color-text-tertiary)]",
                      "hover:bg-[var(--color-hover-bg)]",
                    )}
                    onClick={() => {
                      setSelectedSyncPairId(null);
                      handleCreate();
                    }}
                    type="button"
                    disabled={creating}
                  >
                    Skip & Create
                  </button>
                  <button
                    className={cn(
                      "rounded-md px-3 py-1.5 text-sm font-medium",
                      "bg-[var(--color-accent)] text-white",
                      "hover:opacity-90",
                      "disabled:opacity-50 disabled:cursor-not-allowed",
                    )}
                    onClick={handleCreate}
                    disabled={creating}
                    type="button"
                  >
                    {creating ? "Creating..." : "Create Space"}
                  </button>
                </>
              )}
            </div>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
