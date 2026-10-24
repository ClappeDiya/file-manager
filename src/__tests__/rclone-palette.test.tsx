import { describe, expect, it, vi } from "vitest";
import { getDefaultCommands } from "@/components/command-palette";

describe("command palette × rclone", () => {
  it("opens the rclone remote manager and the rclone settings", () => {
    const onOpenRcloneRemotes = vi.fn();
    const onOpenSettings = vi.fn();
    const cmds = getDefaultCommands({ onOpenRcloneRemotes, onOpenSettings });
    const remotes = cmds.find((c) => c.id === "open-rclone-remotes");
    expect(remotes?.label).toBe("rclone Remotes…");
    remotes?.action();
    expect(onOpenRcloneRemotes).toHaveBeenCalledOnce();
    const settings = cmds.find((c) => c.id === "rclone-settings");
    settings?.action();
    expect(onOpenSettings).toHaveBeenCalledOnce();
    expect(settings?.keywords).toContain("rclone.conf");
  });

  it("opens Settings on the rclone section when the host supports it", () => {
    const onOpenSettings = vi.fn();
    const onOpenRcloneSettings = vi.fn();
    const settings = getDefaultCommands({ onOpenSettings, onOpenRcloneSettings }).find((c) => c.id === "rclone-settings");
    settings?.action();
    expect(onOpenRcloneSettings).toHaveBeenCalledOnce();
    expect(onOpenSettings).not.toHaveBeenCalled();
  });
});
