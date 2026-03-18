/**
 * Archive Browser Component (T-062)
 *
 * Features:
 * - Double-click archive opens as virtual folder
 * - Browse archive contents with breadcrumb navigation
 * - Extract: right-click > Extract Here / Extract To
 * - Create: ZIP, TAR, TAR.GZ, 7Z
 * - Password-protected ZIP/7Z
 * - Large archives use queue with progress
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

interface ArchiveEntry {
  name: string;
  path: string;
  size: number;
  compressed_size: number | null;
  is_dir: boolean;
  modified: string | null;
  permissions: string | null;
}

interface ArchiveBrowseResult {
  archive_path: string;
  current_dir: string;
  entries: ArchiveEntry[];
  total_files: number;
  total_dirs: number;
  total_size: number;
}

interface CreateArchiveParams {
  output_path: string;
  source_paths: string[];
  format: string;
  password: string | null;
  compression_level: number | null;
}

interface ExtractArchiveParams {
  archive_path: string;
  dest_dir: string | null;
  password: string | null;
  selected_entries: string[] | null;
}

interface ArchiveOpResult {
  success: boolean;
  action: string;
  archive_path: string;
  files_processed: number;
  total_size: number;
  errors: string[];
  queue_id: string | null;
}

type ArchiveFormat = "zip" | "tar" | "tar.gz" | "7z";

interface ArchiveBrowserProps {
  archivePath: string;
  onClose?: () => void;
  onExtractComplete?: (result: ArchiveOpResult) => void;
}

interface CreateArchiveDialogProps {
  sourcePaths: string[];
  isOpen: boolean;
  onClose: () => void;
  onComplete?: (result: ArchiveOpResult) => void;
}

// ── Utility ──

function formatBytes(bytes: number): string {
  if (bytes === 0) return "0 B";
  const k = 1024;
  const sizes = ["B", "KB", "MB", "GB", "TB"];
  const i = Math.floor(Math.log(bytes) / Math.log(k));
  return `${(bytes / Math.pow(k, i)).toFixed(1)} ${sizes[i]}`;
}

function getFileIcon(entry: ArchiveEntry): string {
  if (entry.is_dir) return "folder";
  const ext = entry.name.split(".").pop()?.toLowerCase() || "";
  if (["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp"].includes(ext)) return "image";
  if (["mp3", "wav", "ogg", "flac", "aac"].includes(ext)) return "audio";
  if (["mp4", "mkv", "avi", "mov", "webm"].includes(ext)) return "video";
  if (["pdf"].includes(ext)) return "pdf";
  if (["zip", "tar", "gz", "7z", "rar"].includes(ext)) return "archive";
  if (["js", "ts", "py", "rs", "go", "java", "c", "cpp", "rb", "php"].includes(ext)) return "code";
  if (["txt", "md", "log", "csv"].includes(ext)) return "text";
  return "file";
}

// ── Archive Browser Component ──

interface ArchiveInfoResult {
  path: string;
  format: string;
  total_files: number;
  total_dirs: number;
  total_size: number;
  compressed_size: number;
  compression_ratio: number;
  encrypted: boolean;
  comment: string | null;
}

export function ArchiveBrowser({ archivePath, onClose, onExtractComplete }: ArchiveBrowserProps) {
  const [browseResult, setBrowseResult] = useState<ArchiveBrowseResult | null>(null);
  const [currentPath, setCurrentPath] = useState("");
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [selectedEntries, setSelectedEntries] = useState<Set<string>>(new Set());
  const [isExtracting, setIsExtracting] = useState(false);
  const [extractPassword, setExtractPassword] = useState("");
  const [showPasswordPrompt, setShowPasswordPrompt] = useState(false);
  const [extractDest, _setExtractDest] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);
  const [archiveInfo, setArchiveInfo] = useState<ArchiveInfoResult | null>(null);
  const [showInfo, setShowInfo] = useState(false);
  const [infoLoading, setInfoLoading] = useState(false);

  // Load archive contents
  const loadContents = useCallback(async (subPath: string) => {
    setIsLoading(true);
    setError(null);
    try {
      const result = await tauriInvoke<ArchiveBrowseResult>("archive_browse", {
        archivePath,
        subPath: subPath || null,
      });
      setBrowseResult(result);
      setCurrentPath(subPath);
      setSelectedEntries(new Set());
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsLoading(false);
    }
  }, [archivePath]);

  useEffect(() => {
    loadContents("");
  }, [loadContents]);

  // Navigate into a directory
  const handleNavigate = (entry: ArchiveEntry) => {
    if (entry.is_dir) {
      loadContents(entry.path);
    }
  };

  // Navigate up
  const handleNavigateUp = () => {
    const parts = currentPath.split("/").filter(Boolean);
    parts.pop();
    loadContents(parts.join("/"));
  };

  // Build breadcrumb parts
  const breadcrumbs = currentPath
    ? currentPath.split("/").filter(Boolean)
    : [];

  // Extract selected or all
  const handleExtract = async (extractHere: boolean) => {
    setIsExtracting(true);
    setError(null);
    setSuccessMessage(null);
    try {
      const params: ExtractArchiveParams = {
        archive_path: archivePath,
        dest_dir: extractHere ? null : extractDest,
        password: extractPassword || null,
        selected_entries: selectedEntries.size > 0 ? Array.from(selectedEntries) : null,
      };
      const result = await tauriInvoke<ArchiveOpResult>("archive_extract", { params });
      if (result.success) {
        setSuccessMessage(
          `Extracted ${result.files_processed} item(s)${result.queue_id ? " (queued)" : ""}`
        );
        onExtractComplete?.(result);
      } else {
        setError(result.errors.join("\n"));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsExtracting(false);
      setShowPasswordPrompt(false);
    }
  };

  // Toggle selection
  const toggleSelect = (path: string) => {
    setSelectedEntries((prev) => {
      const next = new Set(prev);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  };

  // Load archive info
  const handleShowInfo = async () => {
    if (showInfo && archiveInfo) {
      setShowInfo(false);
      return;
    }
    setInfoLoading(true);
    setError(null);
    try {
      const info = await tauriInvoke<ArchiveInfoResult>("archive_info", { archivePath });
      setArchiveInfo(info);
      setShowInfo(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setInfoLoading(false);
    }
  };

  const archiveName = archivePath.split("/").pop() || archivePath;

  return (
    <div className="flex flex-col h-full bg-surface-primary text-text-primary">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border-primary bg-surface-secondary">
        <div className="flex items-center gap-2 min-w-0">
          <svg className="w-5 h-5 text-amber-500 shrink-0" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
            <path d="M21 8v13H3V8M1 3h22v5H1z" />
            <path d="M10 12h4" />
          </svg>
          <h2 className="text-sm font-semibold truncate">{archiveName}</h2>
        </div>
        <div className="flex items-center gap-2">
          {browseResult && (
            <span className="text-xs text-text-tertiary">
              {browseResult.total_files} files, {browseResult.total_dirs} dirs
            </span>
          )}
          <button
            onClick={handleShowInfo}
            disabled={infoLoading}
            className={`px-2 py-1 text-xs rounded border ${
              showInfo
                ? "bg-accent-primary text-white border-accent-primary"
                : "border-border-primary hover:bg-surface-hover"
            } disabled:opacity-50`}
            aria-label="Show archive info"
          >
            {infoLoading ? "..." : "Info"}
          </button>
          {onClose && (
            <button onClick={onClose} className="p-1 rounded hover:bg-surface-hover text-text-secondary">
              <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                <path d="M18 6L6 18M6 6l12 12" />
              </svg>
            </button>
          )}
        </div>
      </div>

      {/* Breadcrumb */}
      <div className="flex items-center px-4 py-1.5 border-b border-border-primary text-sm gap-1 bg-surface-primary overflow-x-auto">
        <button
          onClick={() => loadContents("")}
          className="text-accent-primary hover:underline shrink-0"
        >
          {archiveName}
        </button>
        {breadcrumbs.map((part, i) => (
          <span key={i} className="flex items-center gap-1">
            <span className="text-text-tertiary">/</span>
            <button
              onClick={() =>
                loadContents(breadcrumbs.slice(0, i + 1).join("/"))
              }
              className="text-accent-primary hover:underline"
            >
              {part}
            </button>
          </span>
        ))}
      </div>

      {/* Archive Info Panel */}
      {showInfo && archiveInfo && (
        <div className="px-4 py-3 border-b border-border-primary bg-surface-secondary/50 space-y-1">
          <div className="flex items-center justify-between">
            <h3 className="text-sm font-semibold">Archive Info</h3>
            <button
              onClick={() => setShowInfo(false)}
              className="text-xs text-text-tertiary hover:text-text-primary"
            >
              Close
            </button>
          </div>
          <div className="grid grid-cols-2 gap-x-6 gap-y-1 text-xs">
            <div>
              <span className="text-text-tertiary">Format: </span>
              <span className="font-medium">{archiveInfo.format.toUpperCase()}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Files: </span>
              <span className="font-medium">{archiveInfo.total_files}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Directories: </span>
              <span className="font-medium">{archiveInfo.total_dirs}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Total Size: </span>
              <span className="font-medium">{formatBytes(archiveInfo.total_size)}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Compressed: </span>
              <span className="font-medium">{formatBytes(archiveInfo.compressed_size)}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Ratio: </span>
              <span className="font-medium">{(archiveInfo.compression_ratio * 100).toFixed(1)}%</span>
            </div>
            <div>
              <span className="text-text-tertiary">Encrypted: </span>
              <span className={`font-medium ${archiveInfo.encrypted ? "text-amber-600" : ""}`}>
                {archiveInfo.encrypted ? "Yes" : "No"}
              </span>
            </div>
            {archiveInfo.comment && (
              <div className="col-span-2">
                <span className="text-text-tertiary">Comment: </span>
                <span className="font-medium">{archiveInfo.comment}</span>
              </div>
            )}
          </div>
        </div>
      )}

      {/* Toolbar */}
      <div className="flex items-center gap-2 px-4 py-2 border-b border-border-primary bg-surface-secondary">
        {currentPath && (
          <button
            onClick={handleNavigateUp}
            className="px-2 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover flex items-center gap-1"
          >
            <svg className="w-3 h-3" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M19 12H5M12 5l-7 7 7 7" />
            </svg>
            Up
          </button>
        )}
        <div className="flex-1" />
        <button
          onClick={() => handleExtract(true)}
          disabled={isExtracting}
          className="px-3 py-1 text-xs rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
        >
          {isExtracting ? "Extracting..." : "Extract Here"}
        </button>
        <button
          onClick={() => setShowPasswordPrompt(true)}
          className="px-3 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover"
        >
          Extract To...
        </button>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto">
        {isLoading && (
          <div className="flex items-center justify-center h-32">
            <div className="animate-spin h-6 w-6 border-4 border-accent-primary border-t-transparent rounded-full" />
          </div>
        )}

        {error && (
          <div className="p-4 text-sm text-red-600 dark:text-red-400">{error}</div>
        )}

        {successMessage && (
          <div className="px-4 py-2 text-sm text-green-700 dark:text-green-400 bg-green-50 dark:bg-green-900/20">
            {successMessage}
          </div>
        )}

        {!isLoading && browseResult && (
          <div className="divide-y divide-border-primary">
            {browseResult.entries.length === 0 && (
              <div className="p-4 text-sm text-text-tertiary text-center">
                Empty directory
              </div>
            )}
            {browseResult.entries.map((entry) => (
              <div
                key={entry.path}
                className={`flex items-center gap-3 px-4 py-2 hover:bg-surface-hover cursor-pointer ${
                  selectedEntries.has(entry.path) ? "bg-accent-primary/10" : ""
                }`}
                onDoubleClick={() => handleNavigate(entry)}
                onClick={() => toggleSelect(entry.path)}
              >
                <input
                  type="checkbox"
                  checked={selectedEntries.has(entry.path)}
                  onChange={() => toggleSelect(entry.path)}
                  className="shrink-0"
                />
                <FileIcon type={getFileIcon(entry)} />
                <span className="flex-1 text-sm truncate">{entry.name}</span>
                {!entry.is_dir && (
                  <span className="text-xs text-text-tertiary shrink-0">
                    {formatBytes(entry.size)}
                  </span>
                )}
                {entry.is_dir && (
                  <svg className="w-4 h-4 text-text-tertiary" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
                    <path d="M9 18l6-6-6-6" />
                  </svg>
                )}
              </div>
            ))}
          </div>
        )}
      </div>

      {/* Password prompt dialog */}
      {showPasswordPrompt && (
        <div className="border-t border-border-primary bg-surface-secondary px-4 py-3 space-y-2">
          <label className="block text-sm font-medium">
            Password (if encrypted)
            <input
              type="password"
              value={extractPassword}
              onChange={(e) => setExtractPassword(e.target.value)}
              className="mt-1 w-full px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
              placeholder="Leave empty if not encrypted"
            />
          </label>
          <div className="flex gap-2 justify-end">
            <button
              onClick={() => setShowPasswordPrompt(false)}
              className="px-3 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover"
            >
              Cancel
            </button>
            <button
              onClick={() => handleExtract(false)}
              disabled={isExtracting}
              className="px-3 py-1 text-xs rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
            >
              Extract
            </button>
          </div>
        </div>
      )}

      {/* Status bar */}
      <div className="px-4 py-1.5 border-t border-border-primary bg-surface-secondary text-xs text-text-tertiary flex justify-between">
        <span>
          {selectedEntries.size > 0
            ? `${selectedEntries.size} selected`
            : `${browseResult?.entries.length || 0} items`}
        </span>
        {browseResult && (
          <span>{formatBytes(browseResult.total_size)}</span>
        )}
      </div>
    </div>
  );
}

// ── Create Archive Dialog ──

export function CreateArchiveDialog({
  sourcePaths,
  isOpen,
  onClose,
  onComplete,
}: CreateArchiveDialogProps) {
  const [format, setFormat] = useState<ArchiveFormat>("zip");
  const [outputPath, setOutputPath] = useState("");
  const [password, setPassword] = useState("");
  const [compressionLevel, setCompressionLevel] = useState(6);
  const [isCreating, setIsCreating] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<ArchiveOpResult | null>(null);

  useEffect(() => {
    if (sourcePaths.length > 0 && isOpen) {
      const firstPath = sourcePaths[0];
      const dir = firstPath.substring(0, firstPath.lastIndexOf("/"));
      const name = sourcePaths.length === 1
        ? firstPath.split("/").pop()?.split(".")[0] || "archive"
        : "archive";
      const ext = format === "tar.gz" ? "tar.gz" : format;
      setOutputPath(`${dir}/${name}.${ext}`);
    }
  }, [sourcePaths, isOpen, format]);

  const handleCreate = async () => {
    setIsCreating(true);
    setError(null);
    try {
      const params: CreateArchiveParams = {
        output_path: outputPath,
        source_paths: sourcePaths,
        format,
        password: password || null,
        compression_level: compressionLevel,
      };
      const res = await tauriInvoke<ArchiveOpResult>("archive_create", { params });
      setResult(res);
      if (res.success) {
        onComplete?.(res);
      } else {
        setError(res.errors.join("\n"));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsCreating(false);
    }
  };

  if (!isOpen) return null;

  const supportsPassword = format === "zip" || format === "7z";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/50">
      <div className="bg-surface-primary rounded-lg shadow-xl w-[480px] max-w-[90vw]">
        <div className="px-5 py-4 border-b border-border-primary">
          <h3 className="text-lg font-semibold">Create Archive</h3>
          <p className="text-sm text-text-secondary mt-1">
            {sourcePaths.length} item(s) selected
          </p>
        </div>

        <div className="px-5 py-4 space-y-4">
          {/* Format */}
          <div>
            <label className="block text-sm font-medium mb-1">Format</label>
            <div className="flex gap-2">
              {(["zip", "tar", "tar.gz", "7z"] as ArchiveFormat[]).map((f) => (
                <button
                  key={f}
                  onClick={() => setFormat(f)}
                  className={`px-3 py-1.5 text-sm rounded border ${
                    format === f
                      ? "bg-accent-primary text-white border-accent-primary"
                      : "border-border-primary hover:bg-surface-hover"
                  }`}
                >
                  {f.toUpperCase()}
                </button>
              ))}
            </div>
          </div>

          {/* Output path */}
          <div>
            <label className="block text-sm font-medium mb-1">Output File</label>
            <input
              type="text"
              value={outputPath}
              onChange={(e) => setOutputPath(e.target.value)}
              className="w-full px-3 py-2 text-sm border rounded bg-surface-primary border-border-primary"
            />
          </div>

          {/* Password */}
          {supportsPassword && (
            <div>
              <label className="block text-sm font-medium mb-1">
                Password (optional)
              </label>
              <input
                type="password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
                className="w-full px-3 py-2 text-sm border rounded bg-surface-primary border-border-primary"
                placeholder="Leave empty for no encryption"
              />
            </div>
          )}

          {/* Compression level */}
          <div>
            <label className="block text-sm font-medium mb-1">
              Compression: {compressionLevel}
            </label>
            <input
              type="range"
              min="0"
              max="9"
              value={compressionLevel}
              onChange={(e) => setCompressionLevel(parseInt(e.target.value))}
              className="w-full"
            />
            <div className="flex justify-between text-xs text-text-tertiary">
              <span>Fastest</span>
              <span>Best</span>
            </div>
          </div>

          {error && (
            <div className="text-sm text-red-600 dark:text-red-400">{error}</div>
          )}
          {result?.success && (
            <div className="text-sm text-green-700 dark:text-green-400">
              Archive created: {result.files_processed} items
            </div>
          )}
        </div>

        <div className="px-5 py-3 border-t border-border-primary flex justify-end gap-2 bg-surface-secondary rounded-b-lg">
          <button
            onClick={onClose}
            className="px-4 py-1.5 text-sm rounded border border-border-primary hover:bg-surface-hover"
          >
            {result?.success ? "Close" : "Cancel"}
          </button>
          {!result?.success && (
            <button
              onClick={handleCreate}
              disabled={isCreating || !outputPath}
              className="px-4 py-1.5 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
            >
              {isCreating ? "Creating..." : "Create Archive"}
            </button>
          )}
        </div>
      </div>
    </div>
  );
}

// ── File icon helper ──

function FileIcon({ type }: { type: string }) {
  const colors: Record<string, string> = {
    folder: "text-amber-500",
    image: "text-purple-500",
    audio: "text-green-500",
    video: "text-blue-500",
    pdf: "text-red-500",
    archive: "text-amber-600",
    code: "text-cyan-500",
    text: "text-gray-500",
    file: "text-gray-400",
  };

  return (
    <svg
      className={`w-5 h-5 shrink-0 ${colors[type] || colors.file}`}
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
    >
      {type === "folder" ? (
        <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
      ) : (
        <>
          <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
          <path d="M14 2v6h6" />
        </>
      )}
    </svg>
  );
}
