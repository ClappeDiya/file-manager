/**
 * Transfer History Panel (T-018 Frontend)
 *
 * Features:
 * - Searchable transfer history by path, date, status, connection
 * - Export as CSV/JSON
 * - Display checksum verification status
 * - 90-day retention with cleanup
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import { formatBytes } from "@/lib/format-bytes";

interface TransferHistoryRecord {
  id: string;
  source_path: string;
  dest_path: string;
  total_bytes: number;
  transferred_bytes: number;
  status: string;
  started_at: string;
  completed_at: string | null;
  duration_ms: number | null;
  speed_bps: number;
  error_message: string | null;
  connection_id: string | null;
  checksum_verified: boolean;
  checksum_match: boolean | null;
  conflict_policy: string | null;
  conflict_resolution: string | null;
  retry_count: number;
}

function formatDuration(ms: number | null): string {
  if (ms === null) return "--";
  if (ms < 1000) return `${ms}ms`;
  const seconds = Math.floor(ms / 1000);
  if (seconds < 60) return `${seconds}s`;
  const mins = Math.floor(seconds / 60);
  const secs = seconds % 60;
  return `${mins}m ${secs}s`;
}

function formatDate(iso: string): string {
  try {
    return new Date(iso).toLocaleString();
  } catch {
    return iso;
  }
}

function fileName(path: string): string {
  const parts = path.split(/[/\\]/);
  return parts[parts.length - 1] || path;
}

export function TransferHistoryPanel() {
  const [records, setRecords] = useState<TransferHistoryRecord[]>([]);
  const [searchQuery, setSearchQuery] = useState("");
  const [statusFilter, setStatusFilter] = useState<string>("");
  const [loading, setLoading] = useState(false);

  const search = useCallback(async () => {
    setLoading(true);
    try {
      const result = await tauriInvoke<TransferHistoryRecord[]>(
        "search_transfer_history",
        {
          pathContains: searchQuery || null,
          status: statusFilter || null,
          connectionId: null,
          limit: 100,
          offset: 0,
        }
      );
      setRecords(result);
    } catch (err) {
      console.error("Failed to search history:", err);
    } finally {
      setLoading(false);
    }
  }, [searchQuery, statusFilter]);

  useEffect(() => {
    search();
  }, [search]);

  const handleExport = useCallback(
    async (format: string) => {
      try {
        const result = await tauriInvoke<string>("export_transfer_history", {
          format,
          pathContains: searchQuery || null,
          status: statusFilter || null,
        });

        const mimeType =
          format === "csv" ? "text/csv" : "application/json";
        const ext = format === "csv" ? "csv" : "json";
        const blob = new Blob([result], { type: mimeType });
        const url = URL.createObjectURL(blob);
        const a = document.createElement("a");
        a.href = url;
        a.download = `transfer-history.${ext}`;
        a.click();
        URL.revokeObjectURL(url);
      } catch (err) {
        console.error("Failed to export:", err);
      }
    },
    [searchQuery, statusFilter]
  );

  const handleCleanup = useCallback(async () => {
    try {
      const deleted = await tauriInvoke<number>("cleanup_transfer_history", undefined, 0);
      if (deleted > 0) {
        search();
      }
    } catch (err) {
      console.error("Failed to cleanup:", err);
    }
  }, [search]);

  return (
    <div
      className="flex flex-col h-full"
      role="region"
      aria-label="Transfer history"
    >
      {/* Header */}
      <div className="flex items-center justify-between px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <h2 className="text-sm font-semibold">Transfer History</h2>
        <div className="flex items-center gap-1">
          <button
            onClick={() => handleExport("csv")}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
          >
            Export CSV
          </button>
          <button
            onClick={() => handleExport("json")}
            className="text-xs px-2 py-1 rounded hover:bg-gray-200 dark:hover:bg-gray-700"
            type="button"
          >
            Export JSON
          </button>
          <button
            onClick={handleCleanup}
            className="text-xs px-2 py-1 rounded text-red-600 hover:bg-red-50 dark:hover:bg-red-900/30"
            type="button"
            title="Remove records older than retention period"
          >
            Cleanup
          </button>
        </div>
      </div>

      {/* Search/Filter bar */}
      <div className="flex items-center gap-2 px-3 py-2 border-b border-gray-200 dark:border-gray-700">
        <input
          type="text"
          value={searchQuery}
          onChange={(e) => setSearchQuery(e.target.value)}
          placeholder="Search by path..."
          className="flex-1 text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          aria-label="Search transfer history"
        />
        <select
          value={statusFilter}
          onChange={(e) => setStatusFilter(e.target.value)}
          className="text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
          aria-label="Filter by status"
        >
          <option value="">All statuses</option>
          <option value="completed">Completed</option>
          <option value="failed">Failed</option>
          <option value="cancelled">Cancelled</option>
        </select>
      </div>

      {/* History records */}
      <div className="flex-1 overflow-y-auto">
        {loading ? (
          <div className="flex items-center justify-center h-24 text-sm text-gray-400">
            Loading...
          </div>
        ) : records.length === 0 ? (
          <div className="flex items-center justify-center h-24 text-sm text-gray-400">
            No transfer history
          </div>
        ) : (
          <table className="w-full text-xs" role="table">
            <thead>
              <tr className="border-b border-gray-200 dark:border-gray-700 bg-gray-50 dark:bg-gray-800/50">
                <th className="text-left px-3 py-1.5 font-medium">File</th>
                <th className="text-left px-2 py-1.5 font-medium">Status</th>
                <th className="text-right px-2 py-1.5 font-medium">Size</th>
                <th className="text-right px-2 py-1.5 font-medium">Duration</th>
                <th className="text-right px-2 py-1.5 font-medium">Speed</th>
                <th className="text-left px-2 py-1.5 font-medium">Checksum</th>
                <th className="text-left px-2 py-1.5 font-medium">Date</th>
              </tr>
            </thead>
            <tbody>
              {records.map((record) => (
                <tr
                  key={record.id}
                  className="border-b border-gray-100 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-800/50"
                >
                  <td className="px-3 py-1.5">
                    <div className="font-medium truncate max-w-[200px]">
                      {fileName(record.source_path)}
                    </div>
                    <div className="text-gray-400 truncate max-w-[200px]">
                      &rarr; {fileName(record.dest_path)}
                    </div>
                  </td>
                  <td className="px-2 py-1.5">
                    <span
                      className={`${
                        record.status === "completed"
                          ? "text-green-600"
                          : record.status === "failed"
                            ? "text-red-600"
                            : "text-gray-500"
                      }`}
                    >
                      {record.status}
                    </span>
                    {record.retry_count > 0 && (
                      <span className="ml-1 text-gray-400">
                        ({record.retry_count} retries)
                      </span>
                    )}
                  </td>
                  <td className="px-2 py-1.5 text-right">
                    {formatBytes(record.total_bytes)}
                  </td>
                  <td className="px-2 py-1.5 text-right">
                    {formatDuration(record.duration_ms)}
                  </td>
                  <td className="px-2 py-1.5 text-right">
                    {record.speed_bps > 0
                      ? `${formatBytes(record.speed_bps)}/s`
                      : "--"}
                  </td>
                  <td className="px-2 py-1.5">
                    {record.checksum_verified ? (
                      <span
                        className={
                          record.checksum_match
                            ? "text-green-600"
                            : "text-red-600"
                        }
                      >
                        {record.checksum_match ? "Match" : "Mismatch"}
                      </span>
                    ) : (
                      <span className="text-gray-400">--</span>
                    )}
                  </td>
                  <td className="px-2 py-1.5 whitespace-nowrap">
                    {formatDate(record.started_at)}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}

export default TransferHistoryPanel;
