/**
 * Integrity Tools Component (T-063)
 *
 * Features:
 * - Checksum: MD5/SHA-1/SHA-256 via right-click
 * - Verify against provided hash
 * - Duplicate detection: filename/size fast pass then hash
 * - Duplicate view: groups with keep newest/largest/by-path/delete
 * - Tags: color-coded, stored in local DB
 * - Labels: custom text
 * - Smart Folders: saved filter queries
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";

// ── Types ──

interface ChecksumResult {
  path: string;
  algorithm: string;
  hash: string;
  size: number;
  computed_at: string;
}

interface VerifyResult {
  path: string;
  algorithm: string;
  expected: string;
  actual: string;
  matches: boolean;
}

interface DuplicateFile {
  path: string;
  size: number;
  modified: string | null;
  is_selected: boolean;
}

interface DuplicateGroup {
  hash: string;
  size: number;
  files: DuplicateFile[];
  count: number;
}

interface DuplicateScanResult {
  total_scanned: number;
  groups: DuplicateGroup[];
  total_duplicates: number;
  wasted_bytes: number;
  scan_time_ms: number;
}

interface FileTag {
  id: string;
  name: string;
  color: string;
  file_count: number;
}

interface FileLabel {
  path: string;
  tags: string[];
  labels: string[];
}

interface SmartFolder {
  id: string;
  name: string;
  query: SmartFolderQuery;
  icon: string | null;
  created_at: string;
  updated_at: string;
}

interface SmartFolderQuery {
  base_paths: string[];
  name_pattern: string | null;
  extension_filter: string[] | null;
  min_size: number | null;
  max_size: number | null;
  modified_after: string | null;
  modified_before: string | null;
  tags: string[] | null;
  labels: string[] | null;
  include_subdirs: boolean;
}

type IntegrityTab = "checksum" | "duplicates" | "tags" | "smart-folders";

interface IntegrityToolsProps {
  selectedFiles?: string[];
  currentDirectory?: string;
  onClose?: () => void;
}

// ── Utility ──

function truncatePath(path: string, maxLen = 50): string {
  if (path.length <= maxLen) return path;
  const parts = path.split("/");
  if (parts.length <= 3) return path;
  return `.../${parts.slice(-3).join("/")}`;
}

// ── Main Component ──

export function IntegrityTools({
  selectedFiles = [],
  currentDirectory = "",
  onClose,
}: IntegrityToolsProps) {
  const [activeTab, setActiveTab] = useState<IntegrityTab>("checksum");

  return (
    <div className="flex flex-col h-full bg-surface-primary text-text-primary">
      {/* Header */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-border-primary">
        <h2 className="text-lg font-semibold">Integrity Tools</h2>
        {onClose && (
          <button onClick={onClose} className="p-1 rounded hover:bg-surface-hover text-text-secondary">
            <svg className="w-5 h-5" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        )}
      </div>

      {/* Tabs */}
      <div className="flex px-4 py-2 gap-1 border-b border-border-primary bg-surface-secondary">
        {([
          ["checksum", "Checksums"],
          ["duplicates", "Duplicates"],
          ["tags", "Tags & Labels"],
          ["smart-folders", "Smart Folders"],
        ] as [IntegrityTab, string][]).map(([tab, label]) => (
          <button
            key={tab}
            onClick={() => setActiveTab(tab)}
            className={`px-3 py-1.5 text-sm rounded-md transition-colors ${
              activeTab === tab
                ? "bg-accent-primary text-white"
                : "text-text-secondary hover:bg-surface-hover"
            }`}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-auto">
        {activeTab === "checksum" && (
          <ChecksumPanel selectedFiles={selectedFiles} />
        )}
        {activeTab === "duplicates" && (
          <DuplicatePanel currentDirectory={currentDirectory} />
        )}
        {activeTab === "tags" && (
          <TagPanel selectedFiles={selectedFiles} />
        )}
        {activeTab === "smart-folders" && (
          <SmartFolderPanel currentDirectory={currentDirectory} />
        )}
      </div>
    </div>
  );
}

// ── Checksum Panel ──

function ChecksumPanel({ selectedFiles }: { selectedFiles: string[] }) {
  const [algorithm, setAlgorithm] = useState("sha256");
  const [results, setResults] = useState<ChecksumResult[]>([]);
  const [isComputing, setIsComputing] = useState(false);
  const [verifyHash, setVerifyHash] = useState("");
  const [verifyResult, setVerifyResult] = useState<VerifyResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const computeChecksums = async () => {
    if (selectedFiles.length === 0) return;
    setIsComputing(true);
    setError(null);
    const newResults: ChecksumResult[] = [];

    for (const file of selectedFiles) {
      try {
        const result = await tauriInvoke<ChecksumResult>("integrity_checksum", {
          path: file,
          algorithm,
        });
        newResults.push(result);
      } catch (e) {
        setError(`Error computing checksum for ${file}: ${e}`);
      }
    }

    setResults(newResults);
    setIsComputing(false);
  };

  const verifyChecksum = async () => {
    if (selectedFiles.length !== 1 || !verifyHash) return;
    setError(null);
    try {
      const result = await tauriInvoke<VerifyResult>("integrity_verify", {
        path: selectedFiles[0],
        algorithm,
        expectedHash: verifyHash,
      });
      setVerifyResult(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="p-4 space-y-4">
      {/* Algorithm selector */}
      <div>
        <label className="block text-sm font-medium mb-2">Algorithm</label>
        <div className="flex gap-2">
          {["md5", "sha1", "sha256"].map((algo) => (
            <button
              key={algo}
              onClick={() => setAlgorithm(algo)}
              className={`px-3 py-1.5 text-sm rounded border ${
                algorithm === algo
                  ? "bg-accent-primary text-white border-accent-primary"
                  : "border-border-primary hover:bg-surface-hover"
              }`}
            >
              {algo.toUpperCase()}
            </button>
          ))}
        </div>
      </div>

      {/* Compute button */}
      <button
        onClick={computeChecksums}
        disabled={isComputing || selectedFiles.length === 0}
        className="px-4 py-2 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
      >
        {isComputing
          ? "Computing..."
          : `Compute ${algorithm.toUpperCase()} (${selectedFiles.length} file${selectedFiles.length !== 1 ? "s" : ""})`}
      </button>

      {/* Results */}
      {results.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-sm font-medium">Results</h3>
          {results.map((r, i) => (
            <div key={i} className="p-3 bg-surface-secondary rounded text-sm">
              <div className="text-text-tertiary text-xs truncate">{r.path}</div>
              <div className="font-mono text-xs mt-1 break-all select-all">{r.hash}</div>
              <div className="text-xs text-text-tertiary mt-1">
                {formatBytes(r.size)} | {r.algorithm.toUpperCase()}
              </div>
            </div>
          ))}
        </div>
      )}

      {/* Verify section */}
      {selectedFiles.length === 1 && (
        <div className="border-t border-border-primary pt-4 space-y-2">
          <h3 className="text-sm font-medium">Verify Hash</h3>
          <div className="flex gap-2">
            <input
              type="text"
              value={verifyHash}
              onChange={(e) => setVerifyHash(e.target.value)}
              className="flex-1 px-3 py-2 text-sm font-mono border rounded bg-surface-primary border-border-primary"
              placeholder="Paste expected hash..."
            />
            <button
              onClick={verifyChecksum}
              disabled={!verifyHash}
              className="px-3 py-2 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
            >
              Verify
            </button>
          </div>
          {verifyResult && (
            <div
              className={`p-3 rounded text-sm ${
                verifyResult.matches
                  ? "bg-green-50 dark:bg-green-900/20 text-green-700 dark:text-green-400"
                  : "bg-red-50 dark:bg-red-900/20 text-red-700 dark:text-red-400"
              }`}
            >
              {verifyResult.matches ? "Hash verified - OK" : "Hash MISMATCH"}
              {!verifyResult.matches && (
                <div className="mt-1 font-mono text-xs break-all">
                  Expected: {verifyResult.expected}
                  <br />
                  Actual: {verifyResult.actual}
                </div>
              )}
            </div>
          )}
        </div>
      )}

      {error && <div className="text-sm text-red-600 dark:text-red-400">{error}</div>}

      {/* Quick Checksum - standalone path-based tool using compute_checksum */}
      <div className="border-t border-border-primary pt-4 space-y-2">
        <h3 className="text-sm font-medium">Quick Checksum</h3>
        <p className="text-xs text-text-tertiary">
          Compute a checksum for any file path without needing to select it first.
        </p>
        <QuickChecksumTool />
      </div>
    </div>
  );
}

// ── Quick Checksum Tool ──

function QuickChecksumTool() {
  const [path, setPath] = useState("");
  const [algorithm, setAlgorithm] = useState("sha256");
  const [result, setResult] = useState<string | null>(null);
  const [isComputing, setIsComputing] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const compute = async () => {
    if (!path.trim()) return;
    setIsComputing(true);
    setError(null);
    setResult(null);
    try {
      const hash = await tauriInvoke<string>("compute_checksum", {
        path: path.trim(),
        algorithm,
      });
      setResult(hash);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsComputing(false);
    }
  };

  return (
    <div className="space-y-2">
      <input
        type="text"
        value={path}
        onChange={(e) => setPath(e.target.value)}
        className="w-full px-3 py-2 text-sm border rounded bg-surface-primary border-border-primary"
        placeholder="/path/to/file"
        onKeyDown={(e) => e.key === "Enter" && compute()}
      />
      <div className="flex gap-2 items-center">
        <div className="flex gap-1">
          {["md5", "sha1", "sha256"].map((algo) => (
            <button
              key={algo}
              onClick={() => setAlgorithm(algo)}
              className={`px-2 py-1 text-xs rounded border ${
                algorithm === algo
                  ? "bg-accent-primary text-white border-accent-primary"
                  : "border-border-primary hover:bg-surface-hover"
              }`}
            >
              {algo.toUpperCase()}
            </button>
          ))}
        </div>
        <button
          onClick={compute}
          disabled={isComputing || !path.trim()}
          className="px-3 py-1 text-xs rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
        >
          {isComputing ? "Computing..." : "Compute"}
        </button>
      </div>
      {result && (
        <div className="p-3 bg-surface-secondary rounded">
          <div className="font-mono text-xs break-all select-all">{result}</div>
          <div className="text-xs text-text-tertiary mt-1">{algorithm.toUpperCase()}</div>
        </div>
      )}
      {error && <div className="text-xs text-red-600 dark:text-red-400">{error}</div>}
    </div>
  );
}

// ── Duplicate Panel ──

function DuplicatePanel({ currentDirectory }: { currentDirectory: string }) {
  const [scanResult, setScanResult] = useState<DuplicateScanResult | null>(null);
  const [isScanning, setIsScanning] = useState(false);
  const [minSize, setMinSize] = useState(1024); // 1KB default
  const [error, setError] = useState<string | null>(null);
  const [expandedGroups, setExpandedGroups] = useState<Set<string>>(new Set());
  const [resolving, setResolving] = useState<string | null>(null);
  const [selectedKeepPaths, setSelectedKeepPaths] = useState<Record<string, string>>({});
  const [resolveMessage, setResolveMessage] = useState<string | null>(null);

  const startScan = async () => {
    if (!currentDirectory) return;
    setIsScanning(true);
    setError(null);
    try {
      const result = await tauriInvoke<DuplicateScanResult>("integrity_find_duplicates", {
        directory: currentDirectory,
        minSize: minSize,
      });
      setScanResult(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setIsScanning(false);
    }
  };

  const toggleGroup = (hash: string) => {
    setExpandedGroups((prev) => {
      const next = new Set(prev);
      if (next.has(hash)) next.delete(hash);
      else next.add(hash);
      return next;
    });
  };

  const handleResolveDuplicates = async (groupHash: string, action: string) => {
    setResolving(groupHash);
    setError(null);
    setResolveMessage(null);
    try {
      const keepPath = action === "keep_selected" ? (selectedKeepPaths[groupHash] || "") : "";
      const result = await tauriInvoke<{ resolved_count: number; freed_bytes: number }>(
        "integrity_resolve_duplicates",
        { groupHash, keepPath, action }
      );
      setResolveMessage(
        `Resolved: ${result.resolved_count} file(s) removed, ${formatBytes(result.freed_bytes)} freed.`
      );
      // Remove the resolved group from scan results
      if (scanResult) {
        setScanResult({
          ...scanResult,
          groups: scanResult.groups.filter((g) => g.hash !== groupHash),
          total_duplicates: scanResult.total_duplicates - result.resolved_count,
          wasted_bytes: scanResult.wasted_bytes - result.freed_bytes,
        });
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setResolving(null);
    }
  };

  return (
    <div className="p-4 space-y-4">
      <div className="flex items-end gap-4">
        <div>
          <label className="block text-sm font-medium mb-1">Directory</label>
          <input
            type="text"
            value={currentDirectory}
            readOnly
            className="px-3 py-2 text-sm border rounded bg-surface-secondary border-border-primary w-64"
          />
        </div>
        <div>
          <label className="block text-sm font-medium mb-1">Min Size</label>
          <select
            value={minSize}
            onChange={(e) => setMinSize(parseInt(e.target.value))}
            className="px-3 py-2 text-sm border rounded bg-surface-primary border-border-primary"
          >
            <option value="1">1 B</option>
            <option value="1024">1 KB</option>
            <option value="10240">10 KB</option>
            <option value="102400">100 KB</option>
            <option value="1048576">1 MB</option>
            <option value="10485760">10 MB</option>
          </select>
        </div>
        <button
          onClick={startScan}
          disabled={isScanning || !currentDirectory}
          className="px-4 py-2 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
        >
          {isScanning ? "Scanning..." : "Find Duplicates"}
        </button>
      </div>

      {error && <div className="text-sm text-red-600 dark:text-red-400">{error}</div>}

      {resolveMessage && (
        <div className="p-3 bg-green-50 dark:bg-green-900/20 rounded text-sm text-green-700 dark:text-green-400">
          {resolveMessage}
        </div>
      )}

      {scanResult && (
        <div className="space-y-3">
          {/* Summary */}
          <div className="p-3 bg-surface-secondary rounded text-sm flex gap-6">
            <div>
              <span className="text-text-tertiary">Scanned: </span>
              <span className="font-medium">{scanResult.total_scanned}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Groups: </span>
              <span className="font-medium">{scanResult.groups.length}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Duplicates: </span>
              <span className="font-medium">{scanResult.total_duplicates}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Wasted: </span>
              <span className="font-medium text-amber-600">{formatBytes(scanResult.wasted_bytes)}</span>
            </div>
            <div>
              <span className="text-text-tertiary">Time: </span>
              <span className="font-medium">{scanResult.scan_time_ms}ms</span>
            </div>
          </div>

          {/* Groups */}
          {scanResult.groups.length === 0 && (
            <div className="p-4 text-center text-text-tertiary text-sm">
              No duplicates found
            </div>
          )}

          {scanResult.groups.map((group) => (
            <div key={group.hash} className="border border-border-primary rounded overflow-hidden">
              <button
                onClick={() => toggleGroup(group.hash)}
                className="w-full flex items-center justify-between px-4 py-2 bg-surface-secondary hover:bg-surface-hover"
              >
                <span className="text-sm flex items-center gap-2">
                  <span className="font-mono text-xs text-text-tertiary">
                    {group.hash.slice(0, 12)}...
                  </span>
                  <span>{group.count} files</span>
                  <span className="text-text-tertiary">({formatBytes(group.size)} each)</span>
                </span>
                <svg
                  className={`w-4 h-4 transition-transform ${expandedGroups.has(group.hash) ? "rotate-90" : ""}`}
                  viewBox="0 0 24 24"
                  fill="none"
                  stroke="currentColor"
                  strokeWidth="2"
                >
                  <path d="M9 18l6-6-6-6" />
                </svg>
              </button>
              {expandedGroups.has(group.hash) && (
                <div className="divide-y divide-border-primary">
                  {group.files.map((file, i) => (
                    <div key={i} className="flex items-center gap-3 px-4 py-2 text-sm">
                      <input
                        type="radio"
                        name={`keep-${group.hash}`}
                        checked={selectedKeepPaths[group.hash] === file.path}
                        onChange={() =>
                          setSelectedKeepPaths((prev) => ({ ...prev, [group.hash]: file.path }))
                        }
                        className="shrink-0"
                        title="Select to keep this file"
                      />
                      <span className="flex-1 truncate" title={file.path}>
                        {truncatePath(file.path)}
                      </span>
                      <span className="text-xs text-text-tertiary shrink-0">
                        {formatBytes(file.size)}
                      </span>
                      {file.modified && (
                        <span className="text-xs text-text-tertiary shrink-0">
                          {new Date(file.modified).toLocaleDateString()}
                        </span>
                      )}
                    </div>
                  ))}
                  {/* Action buttons */}
                  <div className="flex gap-2 px-4 py-2 bg-surface-secondary flex-wrap">
                    <button
                      onClick={() => handleResolveDuplicates(group.hash, "keep_newest")}
                      disabled={resolving === group.hash}
                      className="px-2 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover disabled:opacity-50"
                    >
                      {resolving === group.hash ? "Resolving..." : "Keep Newest"}
                    </button>
                    <button
                      onClick={() => handleResolveDuplicates(group.hash, "keep_largest")}
                      disabled={resolving === group.hash}
                      className="px-2 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover disabled:opacity-50"
                    >
                      Keep Largest
                    </button>
                    <button
                      onClick={() => handleResolveDuplicates(group.hash, "keep_selected")}
                      disabled={resolving === group.hash || !selectedKeepPaths[group.hash]}
                      className="px-2 py-1 text-xs rounded border border-accent-primary text-accent-primary hover:bg-accent-primary/10 disabled:opacity-50"
                      title={selectedKeepPaths[group.hash] ? `Keep: ${selectedKeepPaths[group.hash]}` : "Select a file to keep first"}
                    >
                      Keep Selected
                    </button>
                    <button
                      onClick={() => {
                        if (confirm(`Delete ALL ${group.count} copies in this group? This cannot be undone.`)) {
                          handleResolveDuplicates(group.hash, "delete_all");
                        }
                      }}
                      disabled={resolving === group.hash}
                      className="px-2 py-1 text-xs rounded border border-red-300 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50"
                    >
                      Delete All
                    </button>
                  </div>
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

// ── Tag Panel ──

function TagPanel({ selectedFiles }: { selectedFiles: string[] }) {
  const [tags, setTags] = useState<FileTag[]>([]);
  const [newTagName, setNewTagName] = useState("");
  const [newTagColor, setNewTagColor] = useState("#3b82f6");
  const [fileInfo, setFileInfo] = useState<FileLabel | null>(null);
  const [newLabel, setNewLabel] = useState("");
  const [error, setError] = useState<string | null>(null);

  const loadTags = useCallback(async () => {
    try {
      const result = await tauriInvoke<FileTag[]>("integrity_list_tags", undefined, []);
      setTags(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  const loadFileInfo = useCallback(async () => {
    if (selectedFiles.length !== 1) {
      setFileInfo(null);
      return;
    }
    try {
      const result = await tauriInvoke<FileLabel>("integrity_get_file_info", {
        path: selectedFiles[0],
      });
      setFileInfo(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, [selectedFiles]);

  useEffect(() => {
    loadTags();
    loadFileInfo();
  }, [loadTags, loadFileInfo]);

  const createTag = async () => {
    if (!newTagName.trim()) return;
    try {
      await tauriInvoke("integrity_create_tag", {
        name: newTagName.trim(),
        color: newTagColor,
      });
      setNewTagName("");
      loadTags();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const deleteTag = async (tagId: string) => {
    try {
      await tauriInvoke("integrity_delete_tag", { tagId });
      loadTags();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const toggleFileTag = async (tagId: string) => {
    if (selectedFiles.length !== 1) return;
    const path = selectedFiles[0];
    try {
      if (fileInfo?.tags.includes(tagId)) {
        await tauriInvoke("integrity_untag_file", { path, tagIds: [tagId] });
      } else {
        await tauriInvoke("integrity_tag_file", { path, tagIds: [tagId] });
      }
      loadFileInfo();
      loadTags();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const addLabel = async () => {
    if (selectedFiles.length !== 1 || !newLabel.trim()) return;
    try {
      await tauriInvoke("integrity_set_label", {
        path: selectedFiles[0],
        label: newLabel.trim(),
      });
      setNewLabel("");
      loadFileInfo();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const removeLabel = async (label: string) => {
    if (selectedFiles.length !== 1) return;
    try {
      await tauriInvoke("integrity_remove_label", {
        path: selectedFiles[0],
        label,
      });
      loadFileInfo();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const PRESET_COLORS = [
    "#ef4444", "#f97316", "#eab308", "#22c55e", "#3b82f6",
    "#8b5cf6", "#ec4899", "#6b7280", "#14b8a6", "#f43f5e",
  ];

  return (
    <div className="p-4 space-y-6">
      {/* Create tag */}
      <div className="space-y-2">
        <h3 className="text-sm font-medium">Tags</h3>
        <div className="flex gap-2 items-center">
          <input
            type="text"
            value={newTagName}
            onChange={(e) => setNewTagName(e.target.value)}
            className="flex-1 px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
            placeholder="New tag name..."
            onKeyDown={(e) => e.key === "Enter" && createTag()}
          />
          <div className="flex gap-1">
            {PRESET_COLORS.map((color) => (
              <button
                key={color}
                onClick={() => setNewTagColor(color)}
                className={`w-5 h-5 rounded-full border-2 ${
                  newTagColor === color ? "border-text-primary" : "border-transparent"
                }`}
                style={{ backgroundColor: color }}
              />
            ))}
          </div>
          <button
            onClick={createTag}
            disabled={!newTagName.trim()}
            className="px-3 py-1.5 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
          >
            Add
          </button>
        </div>

        {/* Tag list */}
        <div className="flex flex-wrap gap-2">
          {tags.map((tag) => (
            <div
              key={tag.id}
              className={`flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs cursor-pointer border ${
                fileInfo?.tags.includes(tag.id)
                  ? "border-current"
                  : "border-transparent hover:border-border-primary"
              }`}
              style={{
                backgroundColor: `${tag.color}20`,
                color: tag.color,
              }}
              onClick={() => toggleFileTag(tag.id)}
            >
              <span
                className="w-2 h-2 rounded-full"
                style={{ backgroundColor: tag.color }}
              />
              {tag.name}
              <span className="text-text-tertiary">({tag.file_count})</span>
              <button
                onClick={(e) => {
                  e.stopPropagation();
                  deleteTag(tag.id);
                }}
                className="ml-1 opacity-50 hover:opacity-100"
              >
                x
              </button>
            </div>
          ))}
          {tags.length === 0 && (
            <span className="text-text-tertiary text-sm">No tags created yet</span>
          )}
        </div>
      </div>

      {/* Labels for selected file */}
      {selectedFiles.length === 1 && (
        <div className="space-y-2 border-t border-border-primary pt-4">
          <h3 className="text-sm font-medium">
            Labels for: {selectedFiles[0].split("/").pop()}
          </h3>
          <div className="flex gap-2">
            <input
              type="text"
              value={newLabel}
              onChange={(e) => setNewLabel(e.target.value)}
              className="flex-1 px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
              placeholder="Add label..."
              onKeyDown={(e) => e.key === "Enter" && addLabel()}
            />
            <button
              onClick={addLabel}
              disabled={!newLabel.trim()}
              className="px-3 py-1.5 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
            >
              Add
            </button>
          </div>
          <div className="flex flex-wrap gap-2">
            {fileInfo?.labels.map((label) => (
              <span
                key={label}
                className="flex items-center gap-1 px-2 py-0.5 bg-surface-secondary rounded text-xs"
              >
                {label}
                <button
                  onClick={() => removeLabel(label)}
                  className="text-text-tertiary hover:text-red-500"
                >
                  x
                </button>
              </span>
            ))}
          </div>
        </div>
      )}

      {error && <div className="text-sm text-red-600 dark:text-red-400">{error}</div>}
    </div>
  );
}

// ── Smart Folder Panel ──

function SmartFolderPanel({ currentDirectory }: { currentDirectory: string }) {
  const [folders, setFolders] = useState<SmartFolder[]>([]);
  const [isCreating, setIsCreating] = useState(false);
  const [newName, setNewName] = useState("");
  const [basePaths, setBasePaths] = useState(currentDirectory);
  const [namePattern, setNamePattern] = useState("");
  const [extFilter, setExtFilter] = useState("");
  const [includeSubdirs, setIncludeSubdirs] = useState(true);
  const [folderResults, setFolderResults] = useState<Record<string, string[]>>({});
  const [error, setError] = useState<string | null>(null);

  const loadFolders = useCallback(async () => {
    try {
      const result = await tauriInvoke<SmartFolder[]>("integrity_list_smart_folders", undefined, []);
      setFolders(result);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  }, []);

  useEffect(() => {
    loadFolders();
  }, [loadFolders]);

  const createFolder = async () => {
    if (!newName.trim() || !basePaths.trim()) return;
    try {
      const query: SmartFolderQuery = {
        base_paths: basePaths.split(",").map((p) => p.trim()),
        name_pattern: namePattern || null,
        extension_filter: extFilter ? extFilter.split(",").map((e) => e.trim()) : null,
        min_size: null,
        max_size: null,
        modified_after: null,
        modified_before: null,
        tags: null,
        labels: null,
        include_subdirs: includeSubdirs,
      };
      await tauriInvoke("integrity_create_smart_folder", {
        name: newName.trim(),
        query,
        icon: null,
      });
      setNewName("");
      setIsCreating(false);
      loadFolders();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const deleteFolder = async (id: string) => {
    try {
      await tauriInvoke("integrity_delete_smart_folder", { folderId: id });
      loadFolders();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  const runFolder = async (id: string) => {
    try {
      const results = await tauriInvoke<string[]>("integrity_run_smart_folder", { folderId: id });
      setFolderResults((prev) => ({ ...prev, [id]: results }));
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <div className="p-4 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">Smart Folders</h3>
        <button
          onClick={() => setIsCreating(!isCreating)}
          className="px-3 py-1.5 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90"
        >
          {isCreating ? "Cancel" : "New Smart Folder"}
        </button>
      </div>

      {/* Create form */}
      {isCreating && (
        <div className="p-4 bg-surface-secondary rounded space-y-3">
          <div>
            <label className="block text-sm font-medium mb-1">Name</label>
            <input
              type="text"
              value={newName}
              onChange={(e) => setNewName(e.target.value)}
              className="w-full px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
              placeholder="Smart folder name..."
            />
          </div>
          <div>
            <label className="block text-sm font-medium mb-1">Base Paths (comma-separated)</label>
            <input
              type="text"
              value={basePaths}
              onChange={(e) => setBasePaths(e.target.value)}
              className="w-full px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
            />
          </div>
          <div className="flex gap-4">
            <div className="flex-1">
              <label className="block text-sm font-medium mb-1">Name Pattern (regex)</label>
              <input
                type="text"
                value={namePattern}
                onChange={(e) => setNamePattern(e.target.value)}
                className="w-full px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
                placeholder=".*\.pdf"
              />
            </div>
            <div className="flex-1">
              <label className="block text-sm font-medium mb-1">Extensions (comma-separated)</label>
              <input
                type="text"
                value={extFilter}
                onChange={(e) => setExtFilter(e.target.value)}
                className="w-full px-3 py-1.5 text-sm border rounded bg-surface-primary border-border-primary"
                placeholder="pdf, docx, txt"
              />
            </div>
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={includeSubdirs}
              onChange={(e) => setIncludeSubdirs(e.target.checked)}
              className="rounded border-border-primary"
            />
            Include subdirectories
          </label>
          <button
            onClick={createFolder}
            disabled={!newName.trim() || !basePaths.trim()}
            className="px-4 py-1.5 text-sm rounded bg-accent-primary text-white hover:bg-accent-primary/90 disabled:opacity-50"
          >
            Create Smart Folder
          </button>
        </div>
      )}

      {/* Folder list */}
      {folders.map((folder) => (
        <div key={folder.id} className="border border-border-primary rounded overflow-hidden">
          <div className="flex items-center justify-between px-4 py-2 bg-surface-secondary">
            <span className="text-sm font-medium">{folder.name}</span>
            <div className="flex gap-2">
              <button
                onClick={() => runFolder(folder.id)}
                className="px-2 py-1 text-xs rounded border border-border-primary hover:bg-surface-hover"
              >
                Run
              </button>
              <button
                onClick={() => deleteFolder(folder.id)}
                className="px-2 py-1 text-xs rounded border border-red-300 text-red-600 hover:bg-red-50 dark:hover:bg-red-900/20"
              >
                Delete
              </button>
            </div>
          </div>
          {folderResults[folder.id] && (
            <div className="p-3 max-h-48 overflow-auto">
              {folderResults[folder.id].length === 0 ? (
                <span className="text-text-tertiary text-sm">No matching files</span>
              ) : (
                <div className="space-y-1">
                  <span className="text-xs text-text-tertiary">
                    {folderResults[folder.id].length} matches
                  </span>
                  {folderResults[folder.id].map((file, i) => (
                    <div key={i} className="text-xs truncate" title={file}>
                      {truncatePath(file, 70)}
                    </div>
                  ))}
                </div>
              )}
            </div>
          )}
        </div>
      ))}

      {folders.length === 0 && !isCreating && (
        <div className="text-center text-text-tertiary text-sm py-8">
          No smart folders created. Click &quot;New Smart Folder&quot; to get started.
        </div>
      )}

      {error && <div className="text-sm text-red-600 dark:text-red-400">{error}</div>}
    </div>
  );
}
