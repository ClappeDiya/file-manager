import { dispatchRefresh } from "@/lib/refresh-affected";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";

export interface TransferOutcome {
  files_transferred: number;
  bytes_transferred: number;
  files_skipped: number;
  verified: boolean;
}

export async function runLocalTransfer(
  source: string,
  destination: string,
  verify: boolean,
  moveSource = false,
): Promise<TransferOutcome> {
  if (!isTauriAvailable())
    throw new Error(
      "File transfers require the desktop app. Open FileManager on your computer and try again.",
    );
  const result = await tauriInvoke<TransferOutcome>("run_local_transfer", {
    source: source.trim(),
    destination: destination.trim(),
    verify,
    moveSource,
  });
  dispatchRefresh([source.trim(), destination.trim()]);
  return result;
}

export function operationError(error: unknown): string {
  if (typeof error === "object" && error !== null && "message" in error) {
    const value = error as { message: unknown; advice?: unknown };
    return [value.message, value.advice].filter(Boolean).join(" ");
  }
  return String(error);
}
