import { useEffect, useRef } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { useTerminalStore } from "@/stores/terminal-store";
import { decodeBase64Bytes } from "@/lib/base64";

interface OutputEvent { session_id: string; data: string; offset: number }
interface ExitEvent { session_id: string }
interface Scrollback { data: string; end_offset: number }

/** xterm.js bound to a backend PTY session (input, output, resize, replay). */
export function XtermView({ sessionId }: { sessionId: string }) {
  const hostRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const host = hostRef.current;
    if (!host) return;
    const term = new Terminal({
      cursorBlink: true,
      fontSize: 12,
      fontFamily: 'ui-monospace, SFMono-Regular, Menlo, Consolas, "Liberation Mono", monospace',
      scrollback: 5000,
      allowProposedApi: false,
      theme: { background: "#1e1e1e", foreground: "#cccccc" },
    });
    const fit = new FitAddon();
    term.loadAddon(fit);
    term.open(host);

    if (!isTauriAvailable()) {
      term.write("The terminal is available in the desktop app.\r\n");
      return () => term.dispose();
    }

    const store = useTerminalStore.getState();
    let disposed = false;
    const unlisten: Array<() => void> = [];
    // Live events that arrive before the scrollback replay finished loading.
    let replayEnd: number | null = null;
    const pending: OutputEvent[] = [];
    const apply = (ev: OutputEvent) => {
      if (replayEnd !== null && ev.offset < replayEnd) return;
      term.write(decodeBase64Bytes(ev.data));
    };

    const doFit = () => {
      try { fit.fit(); } catch { /* host not laid out yet */ }
    };
    doFit();

    term.onData((d) => { void store.writeToSession(sessionId, d); });
    term.onResize(({ cols, rows }) => { void store.resizeSession(sessionId, cols, rows); });

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event");
      const offOut = await listen<OutputEvent>("terminal-output", (e) => {
        if (e.payload.session_id !== sessionId) return;
        if (replayEnd === null) pending.push(e.payload);
        else apply(e.payload);
      });
      const offExit = await listen<ExitEvent>("terminal-exit", (e) => {
        if (e.payload.session_id === sessionId) term.write("\r\n\x1b[2m[process exited]\x1b[0m\r\n");
      });
      if (disposed) { offOut(); offExit(); return; }
      unlisten.push(offOut, offExit);

      try {
        const sb = await tauriInvoke<Scrollback>("terminal_read_scrollback", { sessionId });
        if (disposed) return;
        term.write(decodeBase64Bytes(sb.data));
        replayEnd = sb.end_offset;
      } catch {
        replayEnd = 0;
      }
      pending.splice(0).forEach(apply);
      doFit();
      void store.resizeSession(sessionId, term.cols, term.rows);
    })();

    const ro = new ResizeObserver(doFit);
    ro.observe(host);
    term.focus();

    return () => {
      disposed = true;
      ro.disconnect();
      unlisten.forEach((f) => f());
      term.dispose();
    };
  }, [sessionId]);

  return <div ref={hostRef} className="h-full w-full bg-[#1e1e1e] px-2 py-1" data-testid={`xterm-${sessionId}`} />;
}
