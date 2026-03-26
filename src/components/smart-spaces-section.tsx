import { useEffect, useCallback } from "react";
import { cn } from "@ufop/ui-components";
import { Layers, Plus, Trash2 } from "lucide-react";
import { useSpacesStore, type SmartSpace } from "@/stores/spaces-store";
import { SpaceStatusDot } from "./space-status-dot";

interface SmartSpacesSectionProps {
  expanded: boolean;
  onToggle: () => void;
  onActivateSpace: (space: SmartSpace) => void;
}

export function SmartSpacesSection({
  expanded,
  onToggle,
  onActivateSpace,
}: SmartSpacesSectionProps) {
  const spaces = useSpacesStore((s) => s.spaces);
  const statusMap = useSpacesStore((s) => s.statusMap);
  const loading = useSpacesStore((s) => s.loading);
  const selectedSpaceId = useSpacesStore((s) => s.selectedSpaceId);
  const loadSpaces = useSpacesStore((s) => s.loadSpaces);
  const openWizard = useSpacesStore((s) => s.openWizard);
  const deleteSpace = useSpacesStore((s) => s.deleteSpace);
  const activateSpace = useSpacesStore((s) => s.activateSpace);
  const refreshAllStatuses = useSpacesStore((s) => s.refreshAllStatuses);

  // Load spaces on mount
  useEffect(() => {
    loadSpaces();
  }, [loadSpaces]);

  // Refresh statuses when spaces change
  useEffect(() => {
    if (spaces.length > 0) {
      refreshAllStatuses();
    }
  }, [spaces.length, refreshAllStatuses]);

  const handleActivate = useCallback(
    async (space: SmartSpace) => {
      const result = await activateSpace(space.id);
      if (result) {
        onActivateSpace(result);
      }
    },
    [activateSpace, onActivateSpace],
  );

  const handleContextMenu = useCallback(
    (e: React.MouseEvent, _space: SmartSpace) => {
      e.preventDefault();
    },
    [],
  );

  return (
    <div className="mb-1" data-testid="sidebar-smart-spaces">
      <div className="flex items-center">
        <button
          className={cn(
            "flex items-center gap-2 flex-1 px-3 py-1.5",
            "text-[length:var(--font-size-xs)] font-semibold uppercase tracking-wider",
            "text-[color:var(--color-text-secondary)]",
            "hover:bg-[var(--color-hover-bg)] transition-theme",
            "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={onToggle}
          aria-expanded={expanded}
          aria-label="Smart Spaces section"
        >
          <svg
            className={cn(
              "h-3 w-3 transition-transform",
              expanded && "rotate-90",
            )}
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            strokeWidth="2"
            aria-hidden="true"
          >
            <polyline points="9 18 15 12 9 6" />
          </svg>
          <Layers className="h-4 w-4" aria-hidden="true" />
          Smart Spaces
        </button>
        <button
          className={cn(
            "mr-2 p-0.5 rounded",
            "text-[color:var(--color-text-tertiary)]",
            "hover:text-[color:var(--color-text-primary)] hover:bg-[var(--color-hover-bg)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={() => openWizard()}
          title="New Smart Space"
          aria-label="Create new smart space"
        >
          <Plus className="h-3.5 w-3.5" />
        </button>
      </div>

      {expanded && (
        <div role="group" aria-label="Smart Spaces">
          {loading && spaces.length === 0 ? (
            <p className="px-4 py-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic">
              Loading...
            </p>
          ) : spaces.length === 0 ? (
            <button
              className="w-full px-4 py-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic hover:text-[color:var(--color-text-secondary)] text-left"
              onClick={() => openWizard()}
            >
              Create your first Smart Space...
            </button>
          ) : (
            spaces.map((space) => (
              <SpaceRow
                key={space.id}
                space={space}
                status={statusMap[space.id]?.status}
                isSelected={space.id === selectedSpaceId}
                onActivate={() => handleActivate(space)}
                onDelete={() => deleteSpace(space.id)}
                onContextMenu={(e) => handleContextMenu(e, space)}
              />
            ))
          )}
        </div>
      )}
    </div>
  );
}

// ── Space Row ──

interface SpaceRowProps {
  space: SmartSpace;
  status: import("@/stores/spaces-store").SpaceStatus | undefined;
  isSelected: boolean;
  onActivate: () => void;
  onDelete: () => void;
  onContextMenu: (e: React.MouseEvent) => void;
}

function SpaceRow({
  space,
  status,
  isSelected,
  onActivate,
  onDelete,
  onContextMenu,
}: SpaceRowProps) {
  return (
    <div
      className={cn(
        "group flex items-center gap-2 px-4 py-1 cursor-pointer",
        "text-[length:var(--font-size-sm)]",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
        isSelected && "bg-[var(--color-active-bg)]",
      )}
      onClick={onActivate}
      onContextMenu={onContextMenu}
      role="button"
      tabIndex={0}
      onKeyDown={(e) => {
        if (e.key === "Enter" || e.key === " ") {
          e.preventDefault();
          onActivate();
        }
      }}
      aria-label={`Open ${space.name}`}
    >
      {/* Color dot */}
      <span
        className="inline-block h-2.5 w-2.5 shrink-0 rounded"
        style={{ backgroundColor: space.color }}
        aria-hidden="true"
      />

      {/* Name */}
      <span className="truncate flex-1 text-[color:var(--color-text-primary)]">
        {space.name}
      </span>

      {/* Status dot */}
      <SpaceStatusDot status={status} />

      {/* Delete button (visible on hover) */}
      <button
        className={cn(
          "opacity-0 group-hover:opacity-100 p-0.5 rounded shrink-0",
          "text-[color:var(--color-text-tertiary)]",
          "hover:text-red-500 hover:bg-[var(--color-hover-bg)]",
        )}
        onClick={(e) => {
          e.stopPropagation();
          onDelete();
        }}
        title="Delete space"
        aria-label={`Delete ${space.name}`}
      >
        <Trash2 className="h-3 w-3" />
      </button>
    </div>
  );
}
