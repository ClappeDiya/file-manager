import { describe, expect, it } from "vitest";
import { act, render, screen, fireEvent } from "@testing-library/react";
import { PromptDialogHost } from "@/components/prompt-dialog-host";
import { promptText, confirmText } from "@/lib/prompt-dialog";

describe("promptText", () => {
  it("resolves with the typed value on submit", async () => {
    render(<PromptDialogHost />);
    let p!: Promise<string | null>;
    act(() => { p = promptText("New folder name:", "New Folder"); });
    const input = screen.getByLabelText("New folder name:") as HTMLInputElement;
    expect(input.value).toBe("New Folder");
    fireEvent.change(input, { target: { value: "run2" } });
    fireEvent.click(screen.getByText("OK"));
    expect(await p).toBe("run2");
  });

  it("resolves null on cancel", async () => {
    render(<PromptDialogHost />);
    let p!: Promise<string | null>;
    act(() => { p = promptText("x", "y"); });
    fireEvent.click(screen.getByText("Cancel"));
    expect(await p).toBeNull();
  });
});

describe("confirmText", () => {
  it("resolves true on the confirm button", async () => {
    render(<PromptDialogHost />);
    let p!: Promise<boolean>;
    act(() => { p = confirmText("Delete file.bin?", "Delete", true); });
    fireEvent.click(screen.getByText("Delete"));
    expect(await p).toBe(true);
  });

  it("resolves false on cancel", async () => {
    render(<PromptDialogHost />);
    let p!: Promise<boolean>;
    act(() => { p = confirmText("Sure?", "Yes"); });
    fireEvent.click(screen.getByText("Cancel"));
    expect(await p).toBe(false);
  });
});
