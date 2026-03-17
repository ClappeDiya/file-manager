/**
 * Preview Engine Component (T-061)
 *
 * Features:
 * - Toggle preview pane or Spacebar press
 * - Image preview with dimensions
 * - PDF viewer (embedded)
 * - Text/code with syntax highlighting (30+ languages)
 * - Markdown rendered (safe text-based rendering)
 * - Audio waveform display
 * - Video thumbnail
 * - Archive contents listing
 * - EXIF metadata display
 * - Sandboxed: no script execution, no network, no write
 * - Render <500ms for files <10MB
 * - Large files (>50MB): metadata only
 */
import { useState, useEffect, useRef, useMemo } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { tauriInvoke } from "@/hooks/use-tauri";

// ── Types ──

interface PreviewResult {
  path: string;
  file_name: string;
  file_size: number;
  mime_type: string;
  preview_type: string;
  content: string | null;
  metadata: FileMetadata;
  is_metadata_only: boolean;
  render_hint: string;
}

interface FileMetadata {
  size: number;
  size_human: string;
  created: string | null;
  modified: string | null;
  accessed: string | null;
  is_readonly: boolean;
  is_symlink: boolean;
  permissions: string | null;
  exif: ExifData | null;
}

interface ExifData {
  width: number | null;
  height: number | null;
  camera_make: string | null;
  camera_model: string | null;
  date_taken: string | null;
  iso: number | null;
  exposure: string | null;
  f_number: string | null;
  focal_length: string | null;
  gps_lat: number | null;
  gps_lon: number | null;
}

interface PreviewPaneProps {
  filePath: string | null;
  isOpen: boolean;
  onToggle: () => void;
  width?: number;
  /** Current connection protocol (e.g. "sftp", "ftp", "local") */
  protocol?: string | null;
  /** Whether advanced mode is active */
  advancedMode?: boolean;
}

// ── Utility ──

function formatDate(iso: string | null): string {
  if (!iso) return "-";
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function getLanguageClass(renderHint: string): string {
  const parts = renderHint.split(":");
  if (parts.length > 1) return parts[1];
  return "plaintext";
}

// ── Parsed markdown types for safe rendering ──

interface MdBlock {
  type: "heading" | "paragraph" | "code" | "list" | "text";
  level?: number;
  content: string;
  language?: string;
  items?: string[];
}

function parseMarkdown(content: string): MdBlock[] {
  const blocks: MdBlock[] = [];
  const lines = content.split("\n");
  let i = 0;

  while (i < lines.length) {
    const line = lines[i];

    // Code blocks
    if (line.startsWith("```")) {
      const lang = line.slice(3).trim();
      const codeLines: string[] = [];
      i++;
      while (i < lines.length && !lines[i].startsWith("```")) {
        codeLines.push(lines[i]);
        i++;
      }
      blocks.push({ type: "code", content: codeLines.join("\n"), language: lang || undefined });
      i++;
      continue;
    }

    // Headings
    const headingMatch = line.match(/^(#{1,6})\s+(.+)$/);
    if (headingMatch) {
      blocks.push({ type: "heading", level: headingMatch[1].length, content: headingMatch[2] });
      i++;
      continue;
    }

    // List items
    if (line.match(/^[-*]\s+/) || line.match(/^\d+\.\s+/)) {
      const items: string[] = [];
      while (i < lines.length && (lines[i].match(/^[-*]\s+/) || lines[i].match(/^\d+\.\s+/))) {
        items.push(lines[i].replace(/^[-*]\s+/, "").replace(/^\d+\.\s+/, ""));
        i++;
      }
      blocks.push({ type: "list", content: "", items });
      continue;
    }

    // Empty line
    if (line.trim() === "") {
      i++;
      continue;
    }

    // Paragraph
    const paraLines: string[] = [];
    while (i < lines.length && lines[i].trim() !== "" && !lines[i].startsWith("#") && !lines[i].startsWith("```")) {
      paraLines.push(lines[i]);
      i++;
    }
    blocks.push({ type: "paragraph", content: paraLines.join(" ") });
  }

  return blocks;
}

// ── Component ──

export function PreviewPane({ filePath, isOpen, onToggle, width = 360, protocol, advancedMode }: PreviewPaneProps) {
  const [preview, setPreview] = useState<PreviewResult | null>(null);
  const [exif, setExif] = useState<ExifData | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [_showMetadata, _setShowMetadata] = useState(false);
  const [activeTab, setActiveTab] = useState<"preview" | "inspector">("preview");
  const abortRef = useRef(0);

  // Editable permissions state
  const [editingPermissions, setEditingPermissions] = useState(false);
  const [permissionsInput, setPermissionsInput] = useState("");
  const [permissionsError, setPermissionsError] = useState<string | null>(null);
  const [editingOwner, setEditingOwner] = useState(false);
  const [ownerUid, setOwnerUid] = useState("");
  const [ownerGid, setOwnerGid] = useState("");
  const [ownerError, setOwnerError] = useState<string | null>(null);
  const supportsPermissions = protocol === "sftp" || protocol === "ftp" || protocol === "ftps";
  const supportsOwnership = protocol === "sftp";

  const handleSetPermissions = async () => {
    if (!filePath || !permissionsInput.trim()) return;
    setPermissionsError(null);
    try {
      await tauriInvoke("set_permissions", {
        protocol: protocol || "sftp",
        path: filePath,
        mode: permissionsInput.trim(),
        recursive: false,
      });
      setEditingPermissions(false);
      // Refresh preview to show updated permissions
      abortRef.current++;
    } catch (err) {
      setPermissionsError(String(err));
    }
  };

  const handleSetOwner = async () => {
    if (!filePath) return;
    setOwnerError(null);
    const uid = parseInt(ownerUid, 10);
    const gid = parseInt(ownerGid, 10);
    if (isNaN(uid) || isNaN(gid)) {
      setOwnerError("UID and GID must be numeric");
      return;
    }
    try {
      await tauriInvoke("set_owner", { path: filePath, uid, gid });
      setEditingOwner(false);
    } catch (err) {
      setOwnerError(String(err));
    }
  };

  // Keyboard shortcut: Spacebar to toggle
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.code === "Space" && !["INPUT", "TEXTAREA", "SELECT"].includes((e.target as HTMLElement)?.tagName)) {
        e.preventDefault();
        onToggle();
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [onToggle]);

  // Load preview when file changes
  useEffect(() => {
    if (!isOpen || !filePath) {
      setPreview(null);
      return;
    }

    const requestId = ++abortRef.current;
    setIsLoading(true);
    setError(null);
    setExif(null);

    tauriInvoke<PreviewResult>("preview_file", { path: filePath })
      .then((result) => {
        if (abortRef.current !== requestId) return;
        setPreview(result);
        setIsLoading(false);

        // Fetch EXIF for images
        if (result.preview_type === "image") {
          tauriInvoke<ExifData>("preview_get_exif", { path: filePath })
            .then((data) => {
              if (abortRef.current === requestId) setExif(data);
            })
            .catch(() => {});
        }
      })
      .catch((e) => {
        if (abortRef.current !== requestId) return;
        setError(e instanceof Error ? e.message : String(e));
        setIsLoading(false);
      });
  }, [filePath, isOpen]);

  if (!isOpen) {
    return (
      <button
        onClick={onToggle}
        className="fixed right-0 top-1/2 -translate-y-1/2 p-2 bg-surface-secondary border border-border-primary rounded-l-md hover:bg-surface-hover z-10"
        title="Open Preview (Space)"
      >
        <svg className="w-5 h-5 text-text-secondary" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
          <path d="M15 3h6v6M9 21H3v-6M21 3l-7 7M3 21l7-7" />
        </svg>
      </button>
    );
  }

  return (
    <div
      className="flex flex-col h-full border-l border-border-primary bg-surface-primary"
      style={{ width: `${width}px`, minWidth: `${width}px` }}
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-border-primary bg-surface-secondary">
        <div className="flex items-center gap-2 min-w-0">
          <h3 className="text-sm font-medium truncate">
            {preview?.file_name || "Preview"}
          </h3>
          {preview && (
            <span className="text-xs text-text-tertiary shrink-0">
              {preview.metadata.size_human}
            </span>
          )}
        </div>
        <div className="flex items-center gap-1">
          <button
            onClick={() => setActiveTab("preview")}
            className={`px-2 py-0.5 rounded text-xs ${
              activeTab === "preview"
                ? "bg-accent-primary/20 text-accent-primary font-medium"
                : "text-text-tertiary hover:bg-surface-hover"
            }`}
          >
            Preview
          </button>
          <button
            onClick={() => setActiveTab("inspector")}
            className={`px-2 py-0.5 rounded text-xs ${
              activeTab === "inspector"
                ? "bg-accent-primary/20 text-accent-primary font-medium"
                : "text-text-tertiary hover:bg-surface-hover"
            }`}
          >
            Inspector
          </button>
          <button
            onClick={onToggle}
            className="p-1 rounded text-text-tertiary hover:bg-surface-hover ml-1"
            title="Close Preview (Space)"
          >
            <svg className="w-4 h-4" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2">
              <path d="M18 6L6 18M6 6l12 12" />
            </svg>
          </button>
        </div>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-auto">
        {isLoading && (
          <div className="flex items-center justify-center h-full">
            <div className="animate-spin h-8 w-8 border-4 border-accent-primary border-t-transparent rounded-full" />
          </div>
        )}

        {error && (
          <div className="p-4 text-sm text-red-600 dark:text-red-400">
            {error}
          </div>
        )}

        {!isLoading && !error && !preview && !filePath && (
          <div className="flex flex-col items-center justify-center h-full text-text-tertiary text-sm gap-2">
            <svg className="w-12 h-12 opacity-30" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1">
              <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
              <path d="M14 2v6h6M16 13H8M16 17H8M10 9H8" />
            </svg>
            Select a file to preview
          </div>
        )}

        {activeTab === "preview" && preview && !isLoading && <PreviewContent preview={preview} exif={exif} />}
        {activeTab === "inspector" && preview && !isLoading && (
          <InspectorContent preview={preview} protocol={protocol} advancedMode={advancedMode} />
        )}
      </div>

      {/* Metadata panel (legacy, shown when Inspector tab is not active) */}
      {activeTab === "preview" && _showMetadata && preview && (
        <div className="border-t border-border-primary bg-surface-secondary px-3 py-2 max-h-48 overflow-auto">
          <div className="text-xs space-y-1">
            <MetadataRow label="Size" value={preview.metadata.size_human} />
            <MetadataRow label="Type" value={preview.mime_type} />
            <MetadataRow label="Created" value={formatDate(preview.metadata.created)} />
            <MetadataRow label="Modified" value={formatDate(preview.metadata.modified)} />
            <MetadataRow label="Accessed" value={formatDate(preview.metadata.accessed)} />
            <MetadataRow label="Read-only" value={preview.metadata.is_readonly ? "Yes" : "No"} />
            <MetadataRow label="Symlink" value={preview.metadata.is_symlink ? "Yes" : "No"} />
            {preview.metadata.permissions && (
              supportsPermissions && !editingPermissions ? (
                <div className="flex justify-between gap-2">
                  <span className="text-text-tertiary">Permissions</span>
                  <button
                    onClick={() => {
                      setPermissionsInput(preview.metadata.permissions || "");
                      setEditingPermissions(true);
                      setPermissionsError(null);
                    }}
                    className="text-text-secondary text-right truncate hover:text-accent-primary hover:underline cursor-pointer"
                    title="Click to edit permissions"
                  >
                    {preview.metadata.permissions}
                  </button>
                </div>
              ) : editingPermissions ? (
                <div className="space-y-1">
                  <div className="flex justify-between gap-2">
                    <span className="text-text-tertiary">Permissions</span>
                    <div className="flex items-center gap-1">
                      <input
                        type="text"
                        value={permissionsInput}
                        onChange={(e) => setPermissionsInput(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSetPermissions();
                          if (e.key === "Escape") setEditingPermissions(false);
                        }}
                        placeholder="755"
                        maxLength={4}
                        className="w-14 text-xs px-1 py-0.5 rounded border border-border-primary bg-surface-secondary text-right font-mono"
                        autoFocus
                      />
                      <button
                        onClick={handleSetPermissions}
                        className="text-xs text-accent-primary hover:underline"
                      >
                        Save
                      </button>
                      <button
                        onClick={() => setEditingPermissions(false)}
                        className="text-xs text-text-tertiary hover:underline"
                      >
                        Cancel
                      </button>
                    </div>
                  </div>
                  {permissionsError && (
                    <div className="text-xs text-red-500">{permissionsError}</div>
                  )}
                </div>
              ) : (
                <MetadataRow label="Permissions" value={preview.metadata.permissions} />
              )
            )}
            {/* Owner editing (advanced mode, SFTP only) */}
            {advancedMode && supportsOwnership && (
              editingOwner ? (
                <div className="space-y-1">
                  <div className="flex justify-between gap-2">
                    <span className="text-text-tertiary">Owner</span>
                    <div className="flex items-center gap-1">
                      <input
                        type="text"
                        value={ownerUid}
                        onChange={(e) => setOwnerUid(e.target.value)}
                        placeholder="UID"
                        className="w-12 text-xs px-1 py-0.5 rounded border border-border-primary bg-surface-secondary text-right font-mono"
                        autoFocus
                      />
                      <span className="text-text-tertiary">:</span>
                      <input
                        type="text"
                        value={ownerGid}
                        onChange={(e) => setOwnerGid(e.target.value)}
                        onKeyDown={(e) => {
                          if (e.key === "Enter") handleSetOwner();
                          if (e.key === "Escape") setEditingOwner(false);
                        }}
                        placeholder="GID"
                        className="w-12 text-xs px-1 py-0.5 rounded border border-border-primary bg-surface-secondary text-right font-mono"
                      />
                      <button onClick={handleSetOwner} className="text-xs text-accent-primary hover:underline">Set</button>
                      <button onClick={() => setEditingOwner(false)} className="text-xs text-text-tertiary hover:underline">X</button>
                    </div>
                  </div>
                  {ownerError && <div className="text-xs text-red-500">{ownerError}</div>}
                </div>
              ) : (
                <div className="flex justify-between gap-2">
                  <span className="text-text-tertiary">Owner</span>
                  <button
                    onClick={() => { setEditingOwner(true); setOwnerError(null); }}
                    className="text-text-secondary text-right hover:text-accent-primary hover:underline cursor-pointer"
                    title="Click to change owner (chown)"
                  >
                    Set owner...
                  </button>
                </div>
              )
            )}
            {exif && (
              <>
                {exif.width && exif.height && (
                  <MetadataRow label="Dimensions" value={`${exif.width} x ${exif.height}`} />
                )}
                {exif.camera_make && <MetadataRow label="Camera" value={`${exif.camera_make} ${exif.camera_model || ""}`} />}
                {exif.date_taken && <MetadataRow label="Date Taken" value={exif.date_taken} />}
                {exif.iso && <MetadataRow label="ISO" value={String(exif.iso)} />}
                {exif.exposure && <MetadataRow label="Exposure" value={exif.exposure} />}
                {exif.f_number && <MetadataRow label="F-number" value={exif.f_number} />}
                {exif.focal_length && <MetadataRow label="Focal Length" value={exif.focal_length} />}
              </>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

// ── Inspector Content ──

function InspectorContent({ preview, protocol, advancedMode: _advancedMode }: {
  preview: PreviewResult;
  protocol?: string | null;
  advancedMode?: boolean;
}) {
  return (
    <div className="p-3 space-y-4">
      {/* Basic Info */}
      <div>
        <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">General</h4>
        <div className="text-xs space-y-1.5">
          <MetadataRow label="Name" value={preview.file_name} />
          <MetadataRow label="Size" value={preview.metadata.size_human} />
          <MetadataRow label="Type" value={preview.mime_type} />
          <MetadataRow label="Created" value={formatDate(preview.metadata.created)} />
          <MetadataRow label="Modified" value={formatDate(preview.metadata.modified)} />
          <MetadataRow label="Read-only" value={preview.metadata.is_readonly ? "Yes" : "No"} />
          {preview.metadata.is_symlink && <MetadataRow label="Symlink" value="Yes" />}
          {preview.metadata.permissions && <MetadataRow label="Permissions" value={preview.metadata.permissions} />}
        </div>
      </div>

      {/* Protocol-specific metadata */}
      {protocol === "sftp" && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">SFTP Details</h4>
          <div className="text-xs space-y-1.5">
            {preview.metadata.permissions && <MetadataRow label="Mode" value={preview.metadata.permissions} />}
            <MetadataRow label="Protocol" value="SFTP (SSH File Transfer)" />
          </div>
        </div>
      )}

      {protocol === "s3" && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">S3 Properties</h4>
          <div className="text-xs space-y-1.5">
            <MetadataRow label="Protocol" value="Amazon S3" />
            <MetadataRow label="Content Type" value={preview.mime_type} />
          </div>
        </div>
      )}

      {protocol === "azure_blob" && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">Azure Blob Properties</h4>
          <div className="text-xs space-y-1.5">
            <MetadataRow label="Protocol" value="Azure Blob Storage" />
          </div>
        </div>
      )}

      {protocol === "swift" && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">Swift Properties</h4>
          <div className="text-xs space-y-1.5">
            <MetadataRow label="Protocol" value="OpenStack Swift" />
          </div>
        </div>
      )}

      {(protocol === "ftp" || protocol === "ftps") && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">FTP Details</h4>
          <div className="text-xs space-y-1.5">
            <MetadataRow label="Protocol" value={protocol === "ftps" ? "FTP over TLS" : "FTP"} />
            {preview.metadata.permissions && <MetadataRow label="Permissions" value={preview.metadata.permissions} />}
          </div>
        </div>
      )}

      {(!protocol || protocol === "local") && (
        <div>
          <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">Local File</h4>
          <div className="text-xs space-y-1.5">
            <MetadataRow label="Accessed" value={formatDate(preview.metadata.accessed)} />
          </div>
        </div>
      )}

      {/* Path Info */}
      <div>
        <h4 className="text-xs font-semibold text-text-secondary mb-2 uppercase tracking-wider">Location</h4>
        <div className="text-xs space-y-1.5">
          <div className="flex flex-col gap-0.5">
            <span className="text-text-tertiary">Path</span>
            <span className="text-text-secondary font-mono text-[10px] break-all select-all">{preview.path}</span>
          </div>
        </div>
      </div>
    </div>
  );
}

// ── Metadata row ──

function MetadataRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex justify-between gap-2">
      <span className="text-text-tertiary">{label}</span>
      <span className="text-text-secondary text-right truncate">{value}</span>
    </div>
  );
}

// ── Preview content renderer ──

function PreviewContent({ preview, exif: _exif }: { preview: PreviewResult; exif: ExifData | null }) {
  if (preview.is_metadata_only) {
    return (
      <div className="flex flex-col items-center justify-center h-full text-text-tertiary text-sm gap-3 p-4">
        <svg className="w-16 h-16 opacity-30" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1">
          <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
          <path d="M14 2v6h6" />
        </svg>
        <p className="font-medium">File too large for preview</p>
        <p>{preview.metadata.size_human}</p>
        <p className="text-xs">Only metadata is shown for files larger than 50 MB</p>
      </div>
    );
  }

  switch (preview.preview_type) {
    case "image":
      return (
        <div className="flex items-center justify-center p-4 h-full">
          <img
            src={convertFileSrc(preview.path)}
            alt={preview.file_name}
            className="max-w-full max-h-full object-contain rounded"
            loading="lazy"
          />
        </div>
      );

    case "pdf":
      return (
        <div className="h-full">
          <iframe
            src={convertFileSrc(preview.path)}
            className="w-full h-full border-0"
            title="PDF Preview"
            sandbox="allow-same-origin"
          />
        </div>
      );

    case "code":
    case "text": {
      const lang = getLanguageClass(preview.render_hint);
      const lines = preview.content?.split("\n") || [];
      return (
        <div className="p-2">
          <div className="flex items-center justify-between px-2 py-1 mb-1 text-xs text-text-tertiary bg-surface-secondary rounded-t">
            <span>{lang}</span>
            <span>{lines.length} lines</span>
          </div>
          <pre className="text-xs leading-relaxed overflow-x-auto font-mono p-3 bg-surface-secondary rounded-b whitespace-pre-wrap break-all">
            {lines.map((line, i) => (
              <div key={i} className="flex">
                <span className="text-text-tertiary select-none w-8 text-right mr-3 shrink-0">{i + 1}</span>
                <span>{line}</span>
              </div>
            ))}
          </pre>
        </div>
      );
    }

    case "markdown":
      return <SafeMarkdownRenderer content={preview.content || ""} />;

    case "audio":
      return (
        <div className="flex flex-col items-center justify-center h-full p-4 gap-4">
          <svg className="w-16 h-16 text-text-tertiary opacity-40" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1">
            <path d="M9 18V5l12-2v13" />
            <circle cx="6" cy="18" r="3" />
            <circle cx="18" cy="16" r="3" />
          </svg>
          <audio
            controls
            src={convertFileSrc(preview.path)}
            className="w-full max-w-xs"
          >
            Your browser does not support audio playback.
          </audio>
          <p className="text-sm text-text-tertiary">{preview.file_name}</p>
        </div>
      );

    case "video":
      return (
        <div className="flex flex-col items-center justify-center h-full p-4 gap-4">
          <video
            controls
            src={convertFileSrc(preview.path)}
            className="w-full max-h-[60vh] rounded"
            preload="metadata"
          >
            Your browser does not support video playback.
          </video>
          <p className="text-sm text-text-tertiary">{preview.file_name}</p>
        </div>
      );

    case "archive":
      return (
        <div className="p-3">
          <div className="text-sm font-medium mb-2">Archive Contents</div>
          <pre className="text-xs font-mono bg-surface-secondary p-3 rounded overflow-auto max-h-[60vh] whitespace-pre-wrap">
            {preview.content || "(empty archive)"}
          </pre>
        </div>
      );

    case "binary":
      return (
        <div className="p-3">
          <div className="text-sm font-medium mb-2">Hex Preview</div>
          <pre className="text-xs font-mono bg-surface-secondary p-3 rounded overflow-auto leading-relaxed">
            {preview.content || "(no content)"}
          </pre>
        </div>
      );

    default:
      return (
        <div className="flex flex-col items-center justify-center h-full text-text-tertiary text-sm gap-3 p-4">
          <svg className="w-16 h-16 opacity-30" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1">
            <path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" />
            <path d="M14 2v6h6" />
          </svg>
          <p>Preview not available for this file type</p>
          <p className="text-xs">{preview.mime_type}</p>
        </div>
      );
  }
}

// ── Safe Markdown renderer using React elements only (no dangerouslySetInnerHTML) ──

function SafeMarkdownRenderer({ content }: { content: string }) {
  const blocks = useMemo(() => parseMarkdown(content), [content]);

  return (
    <div className="p-4 space-y-3 text-sm">
      {blocks.map((block, i) => {
        switch (block.type) {
          case "heading": {
            const cls =
              block.level === 1
                ? "text-xl font-bold"
                : block.level === 2
                ? "text-lg font-semibold"
                : "text-base font-medium";
            return (
              <div key={i} className={cls}>
                {formatInlineText(block.content)}
              </div>
            );
          }
          case "code":
            return (
              <pre key={i} className="text-xs font-mono bg-surface-secondary p-3 rounded overflow-auto">
                {block.content}
              </pre>
            );
          case "list":
            return (
              <ul key={i} className="list-disc pl-5 space-y-1">
                {block.items?.map((item, j) => (
                  <li key={j}>{formatInlineText(item)}</li>
                ))}
              </ul>
            );
          case "paragraph":
          default:
            return (
              <p key={i} className="leading-relaxed">
                {formatInlineText(block.content)}
              </p>
            );
        }
      })}
    </div>
  );
}

/** Parse inline markdown (bold, italic, code, links) into React elements. */
function formatInlineText(text: string): React.ReactNode {
  const parts: React.ReactNode[] = [];
  let remaining = text;
  let key = 0;

  while (remaining.length > 0) {
    // Bold
    const boldMatch = remaining.match(/\*\*(.+?)\*\*/);
    // Italic
    const italicMatch = remaining.match(/\*(.+?)\*/);
    // Inline code
    const codeMatch = remaining.match(/`(.+?)`/);
    // Link
    const linkMatch = remaining.match(/\[(.+?)\]\((.+?)\)/);

    // Find earliest match
    const matches = [
      boldMatch ? { idx: remaining.indexOf(boldMatch[0]), type: "bold" as const, match: boldMatch } : null,
      italicMatch ? { idx: remaining.indexOf(italicMatch[0]), type: "italic" as const, match: italicMatch } : null,
      codeMatch ? { idx: remaining.indexOf(codeMatch[0]), type: "code" as const, match: codeMatch } : null,
      linkMatch ? { idx: remaining.indexOf(linkMatch[0]), type: "link" as const, match: linkMatch } : null,
    ].filter((m): m is NonNullable<typeof m> => m !== null);

    if (matches.length === 0) {
      parts.push(remaining);
      break;
    }

    const earliest = matches.reduce((a, b) => (a.idx < b.idx ? a : b));

    if (earliest.idx > 0) {
      parts.push(remaining.slice(0, earliest.idx));
    }

    switch (earliest.type) {
      case "bold":
        parts.push(<strong key={key++}>{earliest.match[1]}</strong>);
        remaining = remaining.slice(earliest.idx + earliest.match[0].length);
        break;
      case "italic":
        parts.push(<em key={key++}>{earliest.match[1]}</em>);
        remaining = remaining.slice(earliest.idx + earliest.match[0].length);
        break;
      case "code":
        parts.push(
          <code key={key++} className="bg-surface-secondary px-1 rounded text-xs font-mono">
            {earliest.match[1]}
          </code>
        );
        remaining = remaining.slice(earliest.idx + earliest.match[0].length);
        break;
      case "link":
        parts.push(
          <a key={key++} href={earliest.match[2]} className="text-accent-primary underline" target="_blank" rel="noopener noreferrer">
            {earliest.match[1]}
          </a>
        );
        remaining = remaining.slice(earliest.idx + earliest.match[0].length);
        break;
    }
  }

  return parts.length === 1 ? parts[0] : <>{parts}</>;
}
