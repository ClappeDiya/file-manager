/**
 * Office document preview (Word, Excel, PowerPoint, OpenDocument, RTF…).
 * The extracted text shows at once; the page itself is rendered by the OS
 * (Quick Look) or LibreOffice into a cached PDF or image
 * (`preview_office_render`). Without a renderer the text stays, with a
 * button to open the file in its own app.
 */
import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { tauriInvoke } from "@/hooks/use-tauri";
import { errorText } from "@/lib/error-text";

export interface OfficeRender {
  kind: "pdf" | "image" | "none";
  path: string | null;
  renderer: string | null;
  message: string | null;
}

export function OfficePreview({
  path,
  fileName,
  text,
}: {
  path: string;
  fileName: string;
  text: string | null;
}) {
  const [render, setRender] = useState<OfficeRender | null>(null);
  const [view, setView] = useState<"page" | "text">("page");
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let live = true;
    setRender(null);
    setError(null);
    setView("page");
    tauriInvoke<OfficeRender>("preview_office_render", { path })
      .then((r) => live && setRender(r))
      .catch((e) => live && setError(errorText(e)));
    return () => {
      live = false;
    };
  }, [path]);

  const openExternally = () =>
    tauriInvoke("open_file_with_default", { path }).catch((e) =>
      setError(errorText(e)),
    );
  const rendered = render && render.kind !== "none" && render.path;
  const showPage = rendered && view === "page";

  return (
    <div className="flex flex-col h-full" data-testid="office-preview">
      <div className="flex items-center gap-2 px-3 py-1.5 border-b border-border-primary text-xs">
        {rendered && text && (
          <div role="tablist" className="flex gap-1">
            {(["page", "text"] as const).map((v) => (
              <button
                key={v}
                role="tab"
                aria-selected={view === v}
                onClick={() => setView(v)}
                className={`px-2 py-0.5 rounded ${view === v ? "bg-accent-primary text-white" : "hover:bg-surface-hover"}`}
              >
                {v === "page" ? "Page" : "Text"}
              </button>
            ))}
          </div>
        )}
        {!render && !error && (
          <span className="text-text-tertiary">Rendering preview…</span>
        )}
        <button
          type="button"
          onClick={openExternally}
          className="ml-auto text-accent-primary hover:underline"
        >
          Open in default app
        </button>
      </div>
      {showPage && render.kind === "pdf" && (
        <iframe
          src={convertFileSrc(render.path!)}
          className="w-full flex-1 border-0"
          title={`${fileName} preview`}
          sandbox="allow-same-origin"
        />
      )}
      {showPage && render.kind === "image" && (
        <div className="flex-1 flex items-center justify-center p-3 min-h-0">
          <img
            src={convertFileSrc(render.path!)}
            alt={fileName}
            className="max-w-full max-h-full object-contain rounded shadow"
          />
        </div>
      )}
      {!showPage && (
        <div className="flex-1 overflow-auto p-3 space-y-2">
          {render?.kind === "none" && render.message && (
            <p className="text-xs text-text-tertiary">{render.message}</p>
          )}
          {error && (
            <p className="text-xs text-red-600 dark:text-red-400">{error}</p>
          )}
          {text ? (
            <pre
              className="text-xs whitespace-pre-wrap font-sans bg-surface-secondary p-3 rounded"
              aria-label="Document text"
            >
              {text}
            </pre>
          ) : (
            render && (
              <p className="text-sm text-text-tertiary">
                No text preview for this format.
              </p>
            )
          )}
        </div>
      )}
    </div>
  );
}
