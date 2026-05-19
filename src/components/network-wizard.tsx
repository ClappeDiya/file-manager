/**
 * Network Configuration Wizard Component
 *
 * Step-by-step network diagnostic tool:
 * 1. Enter hostname/port or select a saved connection
 * 2. Click "Run Diagnostics"
 * 3. Live progress: DNS, Port, TLS, Auth results
 * 4. Expandable details per step
 * 5. Copy Results button for sharing
 *
 * Accessible from Settings > Network > "Run Diagnostics" button.
 */
import { useState, useCallback, useEffect } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { copyToClipboardWithToast } from "@/lib/clipboard";

// ── Types ──

interface TestResult {
  test_name: string;
  passed: boolean;
  duration_ms: number;
  details: string;
  error: string | null;
}

interface ConnectionProfile {
  id: string;
  name: string;
  protocol: string;
  host: string;
  port: number | null;
}

// ── Protocol defaults ──

const PROTOCOL_PORTS: Record<string, number> = {
  sftp: 22,
  ftp: 21,
  ftps: 990,
  webdav: 443,
  smb: 445,
  nfs: 2049,
  s3: 443,
  google_drive: 443,
  dropbox: 443,
  onedrive: 443,
};

// ── Test Step Display ──

function TestStep({
  result,
  running,
  pending,
}: {
  result: TestResult | null;
  running: boolean;
  pending: boolean;
}) {
  const [expanded, setExpanded] = useState(false);

  if (pending && !result && !running) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 text-sm text-gray-400">
        <span className="w-5 h-5 flex items-center justify-center text-xs">--</span>
        <span>Waiting...</span>
      </div>
    );
  }

  if (running && !result) {
    return (
      <div className="flex items-center gap-2 px-3 py-2 text-sm">
        <span className="w-5 h-5 flex items-center justify-center">
          <span className="inline-block w-3 h-3 border-2 border-blue-400 border-t-transparent rounded-full animate-spin" />
        </span>
        <span className="text-blue-500">Running...</span>
      </div>
    );
  }

  if (!result) return null;

  const icon = result.passed ? (
    <span className="w-5 h-5 flex items-center justify-center text-green-500 font-bold text-xs">
      OK
    </span>
  ) : (
    <span className="w-5 h-5 flex items-center justify-center text-red-500 font-bold text-xs">
      !!
    </span>
  );

  return (
    <div className="border-b border-gray-100 dark:border-gray-800 last:border-b-0">
      <button
        type="button"
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-3 py-2 text-sm text-left hover:bg-gray-50 dark:hover:bg-gray-800/50"
      >
        {icon}
        <span className="flex-1 font-medium">{result.test_name}</span>
        <span className="text-xs text-gray-400">
          {result.duration_ms > 0 ? `${result.duration_ms}ms` : ""}
        </span>
        <span className="text-xs text-gray-400">{expanded ? "hide" : "show"}</span>
      </button>
      {expanded && (
        <div className="px-3 pb-2 ml-7 text-xs space-y-1">
          {result.details && (
            <p className="text-gray-600 dark:text-gray-400">{result.details}</p>
          )}
          {result.error && <p className="text-red-500">{result.error}</p>}
        </div>
      )}
    </div>
  );
}

// ── Main Wizard ──

export default function NetworkWizard({
  onBack,
}: {
  onBack?: () => void;
}) {
  // Input state
  const [host, setHost] = useState("");
  const [port, setPort] = useState("");
  const [protocol, setProtocol] = useState("sftp");
  const [connectionId, setConnectionId] = useState("");
  const [connections, setConnections] = useState<ConnectionProfile[]>([]);
  const [useConnection, setUseConnection] = useState(false);

  // Diagnostic state
  const [running, setRunning] = useState(false);
  const [currentStep, setCurrentStep] = useState(-1);
  const [results, setResults] = useState<TestResult[]>([]);

  // Load saved connections
  useEffect(() => {
    tauriInvokeSafe<ConnectionProfile[]>("list_connections", undefined, []).then(
      setConnections,
    );
  }, []);

  // Fill fields from selected connection
  useEffect(() => {
    if (!connectionId || !useConnection) return;
    const conn = connections.find((c) => c.id === connectionId);
    if (conn) {
      setHost(conn.host);
      setPort(String(conn.port ?? PROTOCOL_PORTS[conn.protocol] ?? ""));
      setProtocol(conn.protocol);
    }
  }, [connectionId, useConnection, connections]);

  const effectivePort = parseInt(port, 10) || PROTOCOL_PORTS[protocol] || 22;

  // Run diagnostics step by step
  const runDiagnostics = useCallback(async () => {
    if (!host.trim()) return;

    setRunning(true);
    setResults([]);
    setCurrentStep(0);

    const allResults: TestResult[] = [];

    // Step 1: DNS
    try {
      const dnsResult = await tauriInvoke<TestResult>("network_test_dns", {
        hostname: host.trim(),
      });
      allResults.push(dnsResult);
      setResults([...allResults]);

      if (!dnsResult.passed) {
        // Skip remaining
        allResults.push({
          test_name: "Port Connectivity",
          passed: false,
          duration_ms: 0,
          details: "",
          error: "Skipped: DNS resolution failed",
        });
        allResults.push({
          test_name: "TLS/Security",
          passed: false,
          duration_ms: 0,
          details: "",
          error: "Skipped: DNS resolution failed",
        });
        setResults([...allResults]);
        setCurrentStep(3);
        setRunning(false);
        return;
      }
    } catch (err) {
      allResults.push({
        test_name: "DNS Resolution",
        passed: false,
        duration_ms: 0,
        details: "",
        error: String(err),
      });
      setResults([...allResults]);
      setCurrentStep(3);
      setRunning(false);
      return;
    }

    // Step 2: Port
    setCurrentStep(1);
    try {
      const portResult = await tauriInvoke<TestResult>("network_test_port", {
        host: host.trim(),
        port: effectivePort,
      });
      allResults.push(portResult);
      setResults([...allResults]);

      if (!portResult.passed) {
        allResults.push({
          test_name: "TLS/Security",
          passed: false,
          duration_ms: 0,
          details: "",
          error: "Skipped: Port is not reachable",
        });
        setResults([...allResults]);
        setCurrentStep(3);
        setRunning(false);
        return;
      }
    } catch (err) {
      allResults.push({
        test_name: "Port Connectivity",
        passed: false,
        duration_ms: 0,
        details: "",
        error: String(err),
      });
      setResults([...allResults]);
      setCurrentStep(3);
      setRunning(false);
      return;
    }

    // Step 3: TLS
    setCurrentStep(2);
    try {
      const tlsResult = await tauriInvoke<TestResult>("network_test_tls", {
        host: host.trim(),
        port: effectivePort,
      });
      allResults.push(tlsResult);
      setResults([...allResults]);
    } catch (err) {
      allResults.push({
        test_name: "TLS/Security",
        passed: false,
        duration_ms: 0,
        details: "",
        error: String(err),
      });
      setResults([...allResults]);
    }

    // Step 4: Auth (only if a connection is selected)
    if (useConnection && connectionId) {
      setCurrentStep(3);
      try {
        const authResult = await tauriInvoke<TestResult>("network_test_auth", {
          connectionId,
        });
        allResults.push(authResult);
        setResults([...allResults]);
      } catch (err) {
        allResults.push({
          test_name: "Authentication",
          passed: false,
          duration_ms: 0,
          details: "",
          error: String(err),
        });
        setResults([...allResults]);
      }
    }

    setCurrentStep(allResults.length);
    setRunning(false);
  }, [host, effectivePort, protocol, useConnection, connectionId]);

  // Copy results
  const handleCopy = async () => {
    if (results.length === 0) return;
    const lines = [
      `UFOP Network Diagnostic Results`,
      `Host: ${host}:${effectivePort} (${protocol})`,
      `Date: ${new Date().toISOString()}`,
      ``,
      ...results.map(
        (r) =>
          `${r.passed ? "PASS" : "FAIL"} | ${r.test_name} | ${r.duration_ms}ms${r.details ? ` | ${r.details}` : ""}${r.error ? ` | Error: ${r.error}` : ""}`,
      ),
    ];
    await copyToClipboardWithToast(
      lines.join("\n"),
      "Copy Network Diagnostics",
    );
  };

  const allPassed = results.length > 0 && results.every((r) => r.passed);
  const anyFailed = results.some((r) => !r.passed);

  return (
    <div className="h-full flex flex-col">
      {/* Header */}
      <div className="flex items-center gap-2 px-4 py-3 border-b border-gray-200 dark:border-gray-700">
        {onBack && (
          <button
            type="button"
            onClick={onBack}
            className="text-xs px-2 py-1 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
          >
            Back
          </button>
        )}
        <h2 className="text-sm font-semibold">Network Diagnostics</h2>
      </div>

      {/* Content */}
      <div className="flex-1 overflow-y-auto p-4 space-y-4">
        {/* Connection Selection */}
        {connections.length > 0 && (
          <label className="flex items-center gap-2 text-sm cursor-pointer">
            <input
              type="checkbox"
              checked={useConnection}
              onChange={(e) => setUseConnection(e.target.checked)}
              className="rounded"
            />
            Use saved connection
          </label>
        )}

        {useConnection && connections.length > 0 ? (
          <div>
            <label className="block text-xs font-medium mb-1">Select Connection</label>
            <select
              value={connectionId}
              onChange={(e) => setConnectionId(e.target.value)}
              className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
            >
              <option value="">-- Select --</option>
              {connections.map((c) => (
                <option key={c.id} value={c.id}>
                  {c.name} ({c.host}:{c.port ?? PROTOCOL_PORTS[c.protocol] ?? "?"})
                </option>
              ))}
            </select>
          </div>
        ) : (
          <div className="space-y-2">
            <div>
              <label className="block text-xs font-medium mb-1">Hostname</label>
              <input
                type="text"
                value={host}
                onChange={(e) => setHost(e.target.value)}
                placeholder="example.com or 192.168.1.100"
                className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
              />
            </div>
            <div className="grid grid-cols-2 gap-2">
              <div>
                <label className="block text-xs font-medium mb-1">Protocol</label>
                <select
                  value={protocol}
                  onChange={(e) => {
                    setProtocol(e.target.value);
                    const defaultPort = PROTOCOL_PORTS[e.target.value];
                    if (defaultPort) setPort(String(defaultPort));
                  }}
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                >
                  <option value="sftp">SFTP</option>
                  <option value="ftp">FTP</option>
                  <option value="ftps">FTPS</option>
                  <option value="webdav">WebDAV</option>
                  <option value="smb">SMB</option>
                  <option value="nfs">NFS</option>
                  <option value="s3">S3</option>
                </select>
              </div>
              <div>
                <label className="block text-xs font-medium mb-1">Port</label>
                <input
                  type="number"
                  value={port}
                  onChange={(e) => setPort(e.target.value)}
                  placeholder={String(PROTOCOL_PORTS[protocol] || 22)}
                  className="w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900"
                />
              </div>
            </div>
          </div>
        )}

        {/* Run button */}
        <button
          type="button"
          onClick={runDiagnostics}
          disabled={running || !host.trim()}
          className="w-full text-sm px-4 py-2 rounded bg-blue-500 text-white hover:bg-blue-600 disabled:opacity-50 font-medium"
        >
          {running ? "Running Diagnostics..." : "Run Diagnostics"}
        </button>

        {/* Results */}
        {(results.length > 0 || running) && (
          <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
            <div className="px-3 py-2 bg-gray-50 dark:bg-gray-800/50 border-b border-gray-200 dark:border-gray-700 flex items-center justify-between">
              <span className="text-xs font-medium text-gray-500 dark:text-gray-400 uppercase tracking-wider">
                Results
              </span>
              {results.length > 0 && !running && (
                <span
                  className={`text-xs font-medium ${allPassed ? "text-green-500" : anyFailed ? "text-red-500" : "text-yellow-500"}`}
                >
                  {allPassed
                    ? "All tests passed"
                    : `${results.filter((r) => r.passed).length}/${results.length} passed`}
                </span>
              )}
            </div>

            {/* DNS */}
            <TestStep
              result={results[0] ?? null}
              running={running && currentStep === 0}
              pending={currentStep < 0}
            />
            {/* Port */}
            <TestStep
              result={results[1] ?? null}
              running={running && currentStep === 1}
              pending={currentStep < 1 && !results[1]}
            />
            {/* TLS */}
            <TestStep
              result={results[2] ?? null}
              running={running && currentStep === 2}
              pending={currentStep < 2 && !results[2]}
            />
            {/* Auth (conditional) */}
            {useConnection && connectionId && (
              <TestStep
                result={results[3] ?? null}
                running={running && currentStep === 3}
                pending={currentStep < 3 && !results[3]}
              />
            )}
          </div>
        )}

        {/* Copy button */}
        {results.length > 0 && !running && (
          <button
            type="button"
            onClick={handleCopy}
            className="text-xs px-3 py-1.5 rounded border border-gray-300 dark:border-gray-600 hover:bg-gray-100 dark:hover:bg-gray-800"
          >
            Copy Results to Clipboard
          </button>
        )}
      </div>
    </div>
  );
}
