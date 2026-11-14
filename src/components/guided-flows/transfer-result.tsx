import type { TransferOutcome } from "@/lib/guided-transfer";

export function TransferResult({
  result,
  moved = false,
}: {
  result: TransferOutcome;
  moved?: boolean;
}) {
  return (
    <div
      role="status"
      className="space-y-3 rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] p-4"
    >
      <h3 className="font-semibold">
        {result.files_transferred} files {moved ? "moved" : "copied"}
      </h3>
      <p>{result.bytes_transferred.toLocaleString()} bytes written.</p>
      <p>
        {result.verified
          ? "Verified against the original files."
          : "Verification incomplete or not requested."}
      </p>
      {result.files_skipped > 0 && (
        <p>
          {result.files_skipped} existing files skipped. The backup is
          incomplete. Choose an empty destination to copy every source file.
        </p>
      )}
    </div>
  );
}
