import { useState, useCallback, useEffect } from "react";
import { useFileManagerStore, type FavoriteItem } from "@/stores/file-manager-store";
import { cn } from "@ufop/ui-components";
import { ScrollArea } from "@ufop/ui-components";
import {
  Star,
  HardDrive,
  Clock,
  ChevronRight,
  ChevronDown,
  Folder,
  FolderOpen,
  Trash2,
  TriangleAlert,
  Sparkles,
} from "lucide-react";
import type { FileEntryData } from "./file-list";
import { SmartSpacesSection } from "./smart-spaces-section";
import type { SmartSpace } from "@/stores/spaces-store";
import { tauriInvokeSafe } from "@/hooks/use-tauri";

// ──────────────────────────────────────────────
// Types
// ──────────────────────────────────────────────

interface FrecencyHit {
  path: string;
  last_seen: string;
  hit_count: number;
  score: number;
}

interface DriveInfo {
  name: string;
  path: string;
  totalSpace?: number;
  freeSpace?: number;
  removable?: boolean;
  /** Iter 19: rich hover tooltip pre-computed at the FileManager →
   *  SidebarNav boundary. Format mirrors the iter-16 status-bar
   *  free-space hover ("Macintosh HD: 245 GB free of 500 GB") with
   *  a "name (mount_point)" fallback for drives without space
   *  accounting. Optional so legacy callers fall back to the
   *  original `name (path)` tooltip without breaking. */
  tooltip?: string;
}

interface TreeNode {
  name: string;
  path: string;
  children?: TreeNode[];
  isLoading?: boolean;
}

// ──────────────────────────────────────────────
// SidebarNav component
// ──────────────────────────────────────────────

interface SidebarNavProps {
  onNavigate: (path: string) => void;
  drives?: DriveInfo[];
  onLoadChildren?: (path: string) => Promise<FileEntryData[]>;
  onEjectDrive?: (path: string) => void;
  onActivateSpace?: (space: SmartSpace) => void;
  className?: string;
}

export function SidebarNav({
  onNavigate,
  drives = [],
  onLoadChildren,
  onEjectDrive,
  onActivateSpace,
  className,
}: SidebarNavProps) {
  const favorites = useFileManagerStore((s) => s.favorites);
  const recentLocations = useFileManagerStore((s) => s.recentLocations);
  const addFavorite = useFileManagerStore((s) => s.addFavorite);
  const removeFavorite = useFileManagerStore((s) => s.removeFavorite);

  const [smartLocations, setSmartLocations] = useState<FrecencyHit[]>([]);

  // Fetch frecency-ranked locations from the ledger on mount and every 60s.
  useEffect(() => {
    const fetchSmartLocations = () => {
      tauriInvokeSafe<FrecencyHit[]>(
        "ledger_frecent_paths",
        { limit: 8, dirsOnly: true },
        [],
      ).then(setSmartLocations);
    };
    fetchSmartLocations();
    const interval = setInterval(fetchSmartLocations, 60_000);
    return () => clearInterval(interval);
  }, []);

  const [expandedSections, setExpandedSections] = useState<Set<string>>(
    new Set(["smart-spaces", "smart-locations", "favorites", "devices"]),
  );

  const toggleSection = useCallback((section: string) => {
    setExpandedSections((prev) => {
      const next = new Set(prev);
      if (next.has(section)) {
        next.delete(section);
      } else {
        next.add(section);
      }
      return next;
    });
  }, []);

  // Handle drop on favorites
  const handleFavoriteDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      const path = e.dataTransfer.getData("text/path");
      const name = e.dataTransfer.getData("text/name");
      if (path) {
        addFavorite({
          id: `fav-${Date.now()}`,
          name: name || path.split("/").filter(Boolean).pop() || path,
          path,
        });
      }
    },
    [addFavorite],
  );

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    e.dataTransfer.dropEffect = "link";
  }, []);

  return (
    <ScrollArea
      className={cn(
        "h-full bg-[var(--color-sidebar-bg)] text-[color:var(--color-sidebar-text)]",
        className,
      )}
      data-testid="sidebar-nav"
    >
      <div className="py-2">
        {/* Smart Spaces Section */}
        <SmartSpacesSection
          expanded={expandedSections.has("smart-spaces")}
          onToggle={() => toggleSection("smart-spaces")}
          onActivateSpace={(space) => onActivateSpace?.(space)}
        />

        {/* Smart Locations — frecency-ranked from operation ledger */}
        {smartLocations.length > 0 && (
          <SidebarSection
            title="Smart"
            icon={<Sparkles className="h-4 w-4" aria-hidden="true" />}
            expanded={expandedSections.has("smart-locations")}
            onToggle={() => toggleSection("smart-locations")}
            testId="sidebar-smart-locations"
          >
            {smartLocations.map((loc) => (
              <button
                key={loc.path}
                className={cn(
                  "flex items-center gap-2 w-full px-4 py-1.5 text-left",
                  "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
                  "hover:bg-[var(--color-hover-bg)] transition-theme",
                  "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
                )}
                onClick={() => onNavigate(loc.path)}
                title={`${loc.path} — ${loc.hit_count} operations`}
              >
                <Folder className="h-3.5 w-3.5 text-[color:var(--color-primary)] shrink-0" aria-hidden="true" />
                <span className="truncate">
                  {loc.path.split("/").filter(Boolean).pop() || loc.path}
                </span>
              </button>
            ))}
          </SidebarSection>
        )}

        {/* Favorites Section */}
        <SidebarSection
          title="Favorites"
          icon={<Star className="h-4 w-4" aria-hidden="true" />}
          expanded={expandedSections.has("favorites")}
          onToggle={() => toggleSection("favorites")}
          onDragOver={handleDragOver}
          onDrop={handleFavoriteDrop}
          testId="sidebar-favorites"
        >
          {favorites.length === 0 ? (
            <p className="px-4 py-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic">
              Drag folders here to add favorites
            </p>
          ) : (
            favorites.map((fav) => (
              <FavoriteRow
                key={fav.id}
                item={fav}
                onNavigate={onNavigate}
                onRemove={() => removeFavorite(fav.id)}
              />
            ))
          )}
        </SidebarSection>

        {/* Devices Section */}
        <SidebarSection
          title="Devices"
          icon={<HardDrive className="h-4 w-4" aria-hidden="true" />}
          expanded={expandedSections.has("devices")}
          onToggle={() => toggleSection("devices")}
          testId="sidebar-devices"
        >
          {drives.length === 0 ? (
            <DriveRow
              drive={{ name: "This Computer", path: "/" }}
              onNavigate={onNavigate}
            />
          ) : (
            drives.map((drive) => (
              <DriveRow
                key={drive.path}
                drive={drive}
                onNavigate={onNavigate}
                onEject={drive.removable ? () => onEjectDrive?.(drive.path) : undefined}
              />
            ))
          )}
        </SidebarSection>

        {/* Tree View Section */}
        <SidebarSection
          title="Folders"
          icon={<Folder className="h-4 w-4" aria-hidden="true" />}
          expanded={expandedSections.has("folders")}
          onToggle={() => toggleSection("folders")}
          testId="sidebar-folders"
        >
          {onLoadChildren ? (
            <TreeView
              rootPath="/"
              onNavigate={onNavigate}
              onLoadChildren={onLoadChildren}
            />
          ) : (
            <p className="px-4 py-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
              Connect to browse folders
            </p>
          )}
        </SidebarSection>

        {/* Recent Locations */}
        <SidebarSection
          title="Recent"
          icon={<Clock className="h-4 w-4" aria-hidden="true" />}
          expanded={expandedSections.has("recent")}
          onToggle={() => toggleSection("recent")}
          testId="sidebar-recent"
        >
          {recentLocations.length === 0 ? (
            <p className="px-4 py-2 text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)] italic">
              No recent locations
            </p>
          ) : (
            recentLocations.slice(0, 10).map((loc) => (
              <button
                key={loc.path}
                className={cn(
                  "flex items-center gap-2 w-full px-4 py-1.5 text-left",
                  "text-[length:var(--font-size-xs)] text-[color:var(--color-text-secondary)]",
                  "hover:bg-[var(--color-hover-bg)] transition-theme",
                  "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
                )}
                onClick={() => onNavigate(loc.path)}
                title={loc.path}
              >
                <Folder className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
                <span className="truncate">
                  {loc.path.split("/").filter(Boolean).pop() || loc.path}
                </span>
              </button>
            ))
          )}
        </SidebarSection>
      </div>
    </ScrollArea>
  );
}

// ──────────────────────────────────────────────
// Sidebar Section (collapsible)
// ──────────────────────────────────────────────

interface SidebarSectionProps {
  title: string;
  icon: React.ReactNode;
  expanded: boolean;
  onToggle: () => void;
  children: React.ReactNode;
  onDragOver?: (e: React.DragEvent) => void;
  onDrop?: (e: React.DragEvent) => void;
  testId?: string;
}

function SidebarSection({
  title,
  icon,
  expanded,
  onToggle,
  children,
  onDragOver,
  onDrop,
  testId,
}: SidebarSectionProps) {
  return (
    <div
      className="mb-1"
      onDragOver={onDragOver}
      onDrop={onDrop}
      data-testid={testId}
    >
      <button
        className={cn(
          "flex items-center gap-2 w-full px-3 py-1.5",
          "text-[length:var(--font-size-xs)] font-semibold uppercase tracking-wider",
          "text-[color:var(--color-text-secondary)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
          "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={onToggle}
        aria-expanded={expanded}
        aria-label={`${title} section`}
      >
        {expanded ? (
          <ChevronDown className="h-3 w-3" aria-hidden="true" />
        ) : (
          <ChevronRight className="h-3 w-3" aria-hidden="true" />
        )}
        {icon}
        {title}
      </button>
      {expanded && <div role="group" aria-label={title}>{children}</div>}
    </div>
  );
}

// ──────────────────────────────────────────────
// Favorite row
// ──────────────────────────────────────────────

function FavoriteRow({
  item,
  onNavigate,
  onRemove,
}: {
  item: FavoriteItem;
  onNavigate: (path: string) => void;
  onRemove: () => void;
}) {
  return (
    <div
      className={cn(
        "group flex items-center gap-2 px-4 py-1.5",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
      )}
    >
      <button
        className={cn(
          "flex items-center gap-2 flex-1 min-w-0 text-left",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={() => onNavigate(item.path)}
        title={item.path}
      >
        <Star className="h-3.5 w-3.5 text-[color:var(--color-warning)] shrink-0" aria-hidden="true" />
        <span className="truncate">{item.name}</span>
      </button>
      <button
        className={cn(
          "opacity-0 group-hover:opacity-100 transition-theme",
          "h-4 w-4 flex items-center justify-center rounded-sm",
          "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-error)]",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={onRemove}
        aria-label={`Remove ${item.name} from favorites`}
        title="Remove from favorites"
      >
        <Trash2 className="h-3 w-3" aria-hidden="true" />
      </button>
    </div>
  );
}

// ──────────────────────────────────────────────
// Drive row
// ──────────────────────────────────────────────

function DriveRow({
  drive,
  onNavigate,
  onEject,
}: {
  drive: DriveInfo;
  onNavigate: (path: string) => void;
  onEject?: () => void;
}) {
  const usedPercent =
    drive.totalSpace && drive.freeSpace
      ? ((drive.totalSpace - drive.freeSpace) / drive.totalSpace) * 100
      : undefined;

  return (
    <div
      className={cn(
        "group flex items-center gap-2 px-4 py-1.5",
        "hover:bg-[var(--color-hover-bg)] transition-theme",
      )}
    >
      <button
        className={cn(
          "flex items-center gap-2 flex-1 min-w-0 text-left",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
        )}
        onClick={() => onNavigate(drive.path)}
        title={drive.tooltip ?? `${drive.name} (${drive.path})`}
      >
        <HardDrive className="h-3.5 w-3.5 text-[color:var(--color-text-secondary)] shrink-0" aria-hidden="true" />
        <div className="flex-1 min-w-0">
          <span className="truncate block">{drive.name}</span>
          {usedPercent !== undefined && (
            <div className="mt-0.5 h-1 w-full bg-[var(--color-border)] rounded-full overflow-hidden">
              <div
                className={cn(
                  "h-full rounded-full",
                  usedPercent > 90
                    ? "bg-[var(--color-error)]"
                    : usedPercent > 70
                      ? "bg-[var(--color-warning)]"
                      : "bg-[var(--color-primary)]",
                )}
                style={{ width: `${usedPercent}%` }}
              />
            </div>
          )}
        </div>
      </button>
      {onEject && (
        <button
          className={cn(
            "opacity-0 group-hover:opacity-100 transition-theme",
            "h-5 w-5 flex items-center justify-center rounded-sm",
            "text-[color:var(--color-text-tertiary)] hover:text-[color:var(--color-text)]",
            "focus-visible:outline-2 focus-visible:outline-[var(--color-focus-ring)]",
          )}
          onClick={onEject}
          aria-label={`Eject ${drive.name}`}
          title={`Eject ${drive.name}`}
        >
          <TriangleAlert className="h-3.5 w-3.5" aria-hidden="true" />
        </button>
      )}
    </div>
  );
}

// ──────────────────────────────────────────────
// Tree View (lazy loading)
// ──────────────────────────────────────────────

interface TreeViewProps {
  rootPath: string;
  onNavigate: (path: string) => void;
  onLoadChildren: (path: string) => Promise<FileEntryData[]>;
}

function TreeView({ rootPath, onNavigate, onLoadChildren }: TreeViewProps) {
  const expandedPaths = useFileManagerStore((s) => s.expandedTreePaths);
  const toggleTreeExpanded = useFileManagerStore((s) => s.toggleTreeExpanded);
  const [childrenCache, setChildrenCache] = useState<Record<string, TreeNode[]>>({});
  const [loadingPaths, setLoadingPaths] = useState<Set<string>>(new Set());

  const handleToggle = useCallback(
    async (path: string) => {
      toggleTreeExpanded(path);
      if (!childrenCache[path] && !loadingPaths.has(path)) {
        setLoadingPaths((prev) => new Set([...prev, path]));
        try {
          const entries = await onLoadChildren(path);
          const nodes: TreeNode[] = entries
            .filter((e) => e.is_dir)
            .map((e) => ({
              name: e.name,
              path: e.path,
            }));
          setChildrenCache((prev) => ({ ...prev, [path]: nodes }));
        } finally {
          setLoadingPaths((prev) => {
            const next = new Set(prev);
            next.delete(path);
            return next;
          });
        }
      }
    },
    [childrenCache, loadingPaths, onLoadChildren, toggleTreeExpanded],
  );

  return (
    <div role="tree" aria-label="Folder tree" data-testid="folder-tree">
      <TreeNodeRow
        name={rootPath === "/" ? "Root" : rootPath.split("/").filter(Boolean).pop() || rootPath}
        path={rootPath}
        depth={0}
        expanded={expandedPaths.includes(rootPath)}
        children={childrenCache[rootPath]}
        childrenCache={childrenCache}
        expandedPaths={expandedPaths}
        loadingPaths={loadingPaths}
        isLoading={loadingPaths.has(rootPath)}
        onToggle={handleToggle}
        onNavigate={onNavigate}
      />
    </div>
  );
}

interface TreeNodeRowProps {
  name: string;
  path: string;
  depth: number;
  expanded: boolean;
  children?: TreeNode[];
  childrenCache: Record<string, TreeNode[]>;
  expandedPaths: string[];
  loadingPaths: Set<string>;
  isLoading: boolean;
  onToggle: (path: string) => void;
  onNavigate: (path: string) => void;
}

function TreeNodeRow({
  name,
  path,
  depth,
  expanded,
  children,
  childrenCache,
  expandedPaths,
  loadingPaths,
  isLoading,
  onToggle,
  onNavigate,
}: TreeNodeRowProps) {
  return (
    <div role="treeitem" aria-expanded={expanded} aria-label={name}>
      <div
        className={cn(
          "flex items-center gap-1 py-1 cursor-pointer",
          "text-[length:var(--font-size-xs)] text-[color:var(--color-text)]",
          "hover:bg-[var(--color-hover-bg)] transition-theme",
          "focus-visible:outline-2 focus-visible:outline-inset focus-visible:outline-[var(--color-focus-ring)]",
        )}
        style={{ paddingLeft: `${depth * 16 + 12}px` }}
        onClick={() => onNavigate(path)}
        onKeyDown={(e) => {
          if (e.key === "Enter") onNavigate(path);
          if (e.key === "ArrowRight" && !expanded) {
            e.preventDefault();
            onToggle(path);
          }
          if (e.key === "ArrowLeft" && expanded) {
            e.preventDefault();
            onToggle(path);
          }
        }}
        tabIndex={0}
      >
        <button
          className="h-4 w-4 flex items-center justify-center shrink-0"
          onClick={(e) => {
            e.stopPropagation();
            onToggle(path);
          }}
          aria-label={expanded ? "Collapse" : "Expand"}
        >
          {isLoading ? (
            <span className="h-3 w-3 border border-[var(--color-text-tertiary)] border-t-transparent rounded-full animate-spin" />
          ) : expanded ? (
            <ChevronDown className="h-3 w-3" aria-hidden="true" />
          ) : (
            <ChevronRight className="h-3 w-3" aria-hidden="true" />
          )}
        </button>
        {expanded ? (
          <FolderOpen className="h-3.5 w-3.5 text-[color:var(--color-primary)] shrink-0" aria-hidden="true" />
        ) : (
          <Folder className="h-3.5 w-3.5 text-[color:var(--color-primary)] shrink-0" aria-hidden="true" />
        )}
        <span className="truncate">{name}</span>
      </div>
      {expanded && children && (
        <div role="group">
          {children.map((child) => (
            <TreeNodeRow
              key={child.path}
              name={child.name}
              path={child.path}
              depth={depth + 1}
              expanded={expandedPaths.includes(child.path)}
              children={childrenCache[child.path]}
              childrenCache={childrenCache}
              expandedPaths={expandedPaths}
              loadingPaths={loadingPaths}
              isLoading={loadingPaths.has(child.path)}
              onToggle={onToggle}
              onNavigate={onNavigate}
            />
          ))}
        </div>
      )}
    </div>
  );
}
