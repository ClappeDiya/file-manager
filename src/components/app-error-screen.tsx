export function AppErrorScreen({
  error,
  showDetails = false,
}: {
  error: Error;
  showDetails?: boolean;
}) {
  return (
    <main className="h-full overflow-auto bg-[var(--color-bg)] text-[color:var(--color-text)] p-8 space-y-4">
      <h1 className="text-xl font-semibold">
        FileManager could not display this screen
      </h1>
      <p role="alert">
        Reload the app to reconnect, then check your recent operation results.
      </p>
      <button
        onClick={() => window.location.reload()}
        className="rounded-md bg-[var(--color-primary)] px-4 py-2 text-[color:var(--color-primary-foreground)]"
      >
        Reload FileManager
      </button>
      {showDetails && (
        <details>
          <summary>Development diagnostics</summary>
          <pre className="whitespace-pre-wrap break-all">
            {error.stack || error.message}
          </pre>
        </details>
      )}
    </main>
  );
}
