import { useState, useEffect } from "react";
import { invoke } from "@tauri-apps/api/core";

function App() {
  const [greeting, setGreeting] = useState<string>("");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    invoke<string>("greet", { name: "UFOP" })
      .then((response) => setGreeting(response))
      .catch((err: unknown) =>
        setError(err instanceof Error ? err.message : String(err)),
      );
  }, []);

  return (
    <div className="flex h-full flex-col items-center justify-center gap-4 p-8">
      <h1 className="text-2xl font-bold">Unified File Operations Platform</h1>
      {error ? (
        <p className="text-[var(--color-error)]" role="alert">
          Connection error: {error}
        </p>
      ) : (
        <p className="text-[var(--color-text-secondary)]">{greeting}</p>
      )}
      <p className="text-sm text-[var(--color-text-tertiary)]">
        Tauri 2 + React + TypeScript + Vite
      </p>
    </div>
  );
}

export default App;
