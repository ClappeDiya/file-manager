import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import {
  CopyPresetPicker,
  RobocopyImport,
  RobocopyOptionsSection,
} from "@/components/robocopy-options";
import {
  COPY_PRESETS,
  DEFAULT_COPY_OPTIONS,
  applyPreset,
  countChangedOptions,
  describeExitCode,
} from "@/lib/copy-options";

describe("copy-options helpers", () => {
  it("defaults match the Rust CopyOptions defaults", () => {
    expect(DEFAULT_COPY_OPTIONS.copy_subdirs).toBe(true);
    expect(DEFAULT_COPY_OPTIONS.copy_timestamps).toBe(true);
    expect(DEFAULT_COPY_OPTIONS.threads).toBe(1);
    expect(DEFAULT_COPY_OPTIONS.symlinks).toBe("follow");
    expect(countChangedOptions(DEFAULT_COPY_OPTIONS)).toBe(0);
  });

  it("presets start from defaults and don't stack", () => {
    const move = COPY_PRESETS.find((p) => p.id === "move")!;
    const update = COPY_PRESETS.find((p) => p.id === "update")!;
    expect(applyPreset(move).move_files).toBe(true);
    expect(applyPreset(update).move_files).toBe(false);
    expect(applyPreset(update).exclude_older).toBe(true);
  });

  it("mirror preset uses mirror mode", () => {
    expect(COPY_PRESETS.find((p) => p.id === "mirror")!.mode).toBe("mirror");
  });

  it("describes Robocopy exit codes", () => {
    expect(describeExitCode(0)).toMatch(/in sync/i);
    expect(describeExitCode(1)).toMatch(/copied/i);
    expect(describeExitCode(9)).toMatch(/could not be copied/i);
    expect(describeExitCode(16)).toMatch(/fatal/i);
  });
});

describe("RobocopyOptionsSection", () => {
  it("toggles an option and reports the change", () => {
    const onChange = vi.fn();
    render(
      <RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={onChange} mode="one_way" />,
    );
    fireEvent.click(screen.getByLabelText(/Never replace a newer destination file/));
    expect(onChange).toHaveBeenCalledWith({ ...DEFAULT_COPY_OPTIONS, exclude_older: true });
  });

  it("shows /XX only in mirror mode and /PURGE otherwise", () => {
    const { rerender } = render(
      <RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={vi.fn()} mode="mirror" />,
    );
    expect(screen.getByText(/Keep extra files/)).toBeInTheDocument();
    expect(screen.queryByText(/Delete destination files/)).toBeNull();
    rerender(
      <RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={vi.fn()} mode="one_way" />,
    );
    expect(screen.getByText(/Delete destination files/)).toBeInTheDocument();
  });

  it("hides dependent fields until they apply", () => {
    render(
      <RobocopyOptionsSection options={DEFAULT_COPY_OPTIONS} onChange={vi.fn()} mode="one_way" />,
    );
    expect(screen.queryByText(/Wait between retries/)).toBeNull();
    expect(screen.queryByText(/remove emptied source folders/)).toBeNull();
    expect(screen.queryByText(/Append instead of overwrite/)).toBeNull();
  });

  it("flags an invalid run-hours window", () => {
    render(
      <RobocopyOptionsSection
        options={{ ...DEFAULT_COPY_OPTIONS, run_hours: "late" }}
        onChange={vi.fn()}
        mode="one_way"
      />,
    );
    expect(screen.getByPlaceholderText("2200-0600")).toHaveAttribute("aria-invalid", "true");
  });
});

describe("CopyPresetPicker", () => {
  it("calls onPick and shows the active preset's hint", () => {
    const onPick = vi.fn();
    const { rerender } = render(<CopyPresetPicker activeId={null} onPick={onPick} />);
    fireEvent.click(screen.getByRole("button", { name: "Mirror" }));
    expect(onPick).toHaveBeenCalledWith(COPY_PRESETS.find((p) => p.id === "mirror"));
    rerender(<CopyPresetPicker activeId="mirror" onPick={onPick} />);
    expect(screen.getByText(/\/MIR/)).toBeInTheDocument();
  });
});

describe("RobocopyImport", () => {
  it("starts collapsed and shows a readable error outside Tauri", async () => {
    render(<RobocopyImport onImport={vi.fn()} />);
    fireEvent.click(screen.getByText(/Paste it here/));
    fireEvent.change(screen.getByPlaceholderText(/robocopy C:/), {
      target: { value: "robocopy a b /MIR" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Import" }));
    expect(await screen.findByText(/not available/i)).toBeInTheDocument();
  });
});
