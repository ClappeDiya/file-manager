/**
 * URL Handler Registration Settings
 *
 * Simple toggle to register/unregister UFOP as the system handler
 * for ftp://, sftp://, and s3:// URL schemes.
 *
 * Can be embedded in the settings panel or shown standalone.
 */
import { useState, useEffect, useCallback } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";

export default function UrlHandlerSettings() {
  const [registered, setRegistered] = useState(false);
  const [loading, setLoading] = useState(true);
  const [toggling, setToggling] = useState(false);
  const [error, setError] = useState("");

  const checkStatus = useCallback(async () => {
    try {
      const status = await tauriInvoke<boolean>(
        "is_url_handler_registered",
        undefined,
        false,
      );
      setRegistered(status);
    } catch (err) {
      console.error("Failed to check URL handler status:", err);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    checkStatus();
  }, [checkStatus]);

  const toggleHandler = async () => {
    setToggling(true);
    setError("");
    try {
      if (registered) {
        await tauriInvoke("unregister_url_handlers");
      } else {
        await tauriInvoke("register_url_handlers");
      }
      await checkStatus();
    } catch (err) {
      setError(String(err));
    } finally {
      setToggling(false);
    }
  };

  if (loading) {
    return (
      <div className="p-3 text-sm text-gray-500 dark:text-gray-400">
        Checking URL handler status...
      </div>
    );
  }

  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg overflow-hidden">
      <div className="px-3 py-2 bg-gray-50 dark:bg-gray-800/50 border-b border-gray-200 dark:border-gray-700">
        <h3 className="text-sm font-medium">URL Protocol Handlers</h3>
      </div>

      <div className="p-3 space-y-3">
        <div className="flex items-center justify-between">
          <div className="flex-1 min-w-0 mr-3">
            <div className="text-sm font-medium">
              Handle ftp://, sftp://, s3:// links
            </div>
            <div className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
              When enabled, clicking protocol links in other apps will open them
              in UFOP
            </div>
          </div>
          <button
            onClick={toggleHandler}
            disabled={toggling}
            className={`relative inline-flex h-5 w-9 flex-shrink-0 items-center rounded-full transition-colors ${
              registered
                ? "bg-blue-500"
                : "bg-gray-300 dark:bg-gray-600"
            } disabled:opacity-50`}
            role="switch"
            aria-checked={registered}
            type="button"
          >
            <span
              className={`inline-block h-3.5 w-3.5 transform rounded-full bg-white transition-transform ${
                registered ? "translate-x-4" : "translate-x-0.5"
              }`}
            />
          </button>
        </div>

        {/* Status indicator */}
        <div className="flex items-center gap-1.5 text-xs">
          <span
            className={`w-2 h-2 rounded-full ${
              registered ? "bg-green-500" : "bg-gray-400"
            }`}
          />
          <span className="text-gray-500 dark:text-gray-400">
            {toggling
              ? "Updating..."
              : registered
                ? "Currently registered as default handler"
                : "Not registered"}
          </span>
        </div>

        {/* Supported protocols */}
        <div className="text-xs text-gray-500 dark:text-gray-400">
          <span className="font-medium">Supported protocols: </span>
          <span className="font-mono">ftp://</span>
          {" , "}
          <span className="font-mono">sftp://</span>
          {" , "}
          <span className="font-mono">s3://</span>
        </div>

        {error && (
          <div className="text-xs text-red-500 dark:text-red-400 bg-red-50 dark:bg-red-900/20 rounded px-2 py-1.5">
            {error}
          </div>
        )}
      </div>
    </div>
  );
}
