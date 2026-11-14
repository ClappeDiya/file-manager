/**
 * Engine options form: every control writes the matching `BackupOptions`
 * field (snake_case values the Rust serde enums accept).
 */
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen } from "@testing-library/react";
import { BackupAdvancedOptions } from "@/components/backup/backup-advanced-options";
import { newBackupJob } from "@/stores/backup-store";

describe("BackupAdvancedOptions", () => {
  const opts = newBackupJob().options;

  it("maps controls onto BackupOptions fields", () => {
    const onChange = vi.fn();
    render(<BackupAdvancedOptions options={opts} locked={false} onChange={onChange} />);
    fireEvent.change(screen.getByTestId("opt-volume-format"), { target: { value: "zip" } });
    expect(onChange).toHaveBeenLastCalledWith({ volume_format: "zip" });
    fireEvent.change(screen.getByTestId("opt-usn"), { target: { value: "required" } });
    expect(onChange).toHaveBeenLastCalledWith({ usn_policy: "required" });
    fireEvent.change(screen.getByTestId("opt-control-files"), { target: { value: "/a/manifest.txt\n\n /b/x \n" } });
    expect(onChange).toHaveBeenLastCalledWith({ control_files: ["/a/manifest.txt", "/b/x"] });
    fireEvent.click(screen.getByLabelText("Create PAR2 recovery files"));
    expect(onChange).toHaveBeenLastCalledWith({ par2: { ...opts.par2, enabled: true } });
    fireEvent.click(screen.getByLabelText("Disable capacity warnings"));
    expect(onChange).toHaveBeenLastCalledWith({ quota_disable: true });
  });

  it("locks the container format once versions exist", () => {
    render(<BackupAdvancedOptions options={opts} locked onChange={() => {}} />);
    expect(screen.getByTestId("opt-volume-format")).toBeDisabled();
  });

  it("defaults match the engine defaults", () => {
    expect(opts.upload_concurrency).toBe(2);
    expect(opts.par2).toEqual({ enabled: false, redundancy_percent: 10, binary: "par2" });
    expect(opts.backup_xattrs && opts.backup_acls).toBe(true);
    expect(opts.usn_policy).toBe("off");
  });
});
