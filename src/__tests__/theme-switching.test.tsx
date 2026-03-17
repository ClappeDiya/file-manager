import { describe, it, expect, beforeEach } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useUIStore } from "@/stores/ui-store";
import { ThemeSwitcher } from "@/components/theme-switcher";

describe("Theme Switching (T-014)", () => {
  beforeEach(() => {
    // Reset store
    useUIStore.setState({
      theme: "system",
      resolvedTheme: "light",
    });
    localStorage.clear();
    document.documentElement.removeAttribute("data-theme");
    document.documentElement.style.colorScheme = "";
  });

  describe("ThemeSwitcher component", () => {
    it("renders theme options as radio group", () => {
      render(<ThemeSwitcher />);
      const radioGroup = screen.getByRole("radiogroup");
      expect(radioGroup).toHaveAttribute("aria-label", "Theme selection");
    });

    it("renders all four theme options", () => {
      render(<ThemeSwitcher />);
      expect(
        screen.getByRole("radio", { name: "Light theme" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("radio", { name: "Dark theme" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("radio", { name: "System theme" }),
      ).toBeInTheDocument();
      expect(
        screen.getByRole("radio", { name: "High Contrast theme" }),
      ).toBeInTheDocument();
    });

    it("selects current theme correctly", () => {
      useUIStore.setState({ theme: "dark", resolvedTheme: "dark" });
      render(<ThemeSwitcher />);
      const darkRadio = screen.getByRole("radio", { name: "Dark theme" });
      expect(darkRadio).toHaveAttribute("aria-checked", "true");
    });

    it("switches theme on click", async () => {
      const user = userEvent.setup();
      render(<ThemeSwitcher />);

      await user.click(
        screen.getByRole("radio", { name: "Dark theme" }),
      );
      expect(useUIStore.getState().theme).toBe("dark");
      expect(useUIStore.getState().resolvedTheme).toBe("dark");
    });

    it("applies data-theme attribute instantly", async () => {
      const user = userEvent.setup();
      render(<ThemeSwitcher />);

      await user.click(
        screen.getByRole("radio", { name: "Dark theme" }),
      );
      expect(document.documentElement.getAttribute("data-theme")).toBe("dark");

      await user.click(
        screen.getByRole("radio", { name: "Light theme" }),
      );
      expect(document.documentElement.getAttribute("data-theme")).toBe("light");
    });

    it("applies high-contrast theme", async () => {
      const user = userEvent.setup();
      render(<ThemeSwitcher />);

      await user.click(
        screen.getByRole("radio", { name: "High Contrast theme" }),
      );
      expect(document.documentElement.getAttribute("data-theme")).toBe(
        "high-contrast",
      );
    });

    it("persists theme choice to localStorage", async () => {
      const user = userEvent.setup();
      render(<ThemeSwitcher />);

      await user.click(
        screen.getByRole("radio", { name: "Dark theme" }),
      );

      const stored = JSON.parse(
        localStorage.getItem("ufop-ui-state") || "{}",
      );
      expect(stored.state.theme).toBe("dark");
    });
  });

  describe("Instant theme switching (no flash)", () => {
    it("setTheme synchronously updates data-theme", () => {
      const { setTheme } = useUIStore.getState();

      // Synchronous: no requestAnimationFrame or setTimeout
      setTheme("dark");
      // Immediately after calling setTheme, data-theme should be set
      expect(document.documentElement.getAttribute("data-theme")).toBe("dark");
    });

    it("setTheme synchronously updates resolvedTheme", () => {
      const { setTheme } = useUIStore.getState();

      setTheme("dark");
      expect(useUIStore.getState().resolvedTheme).toBe("dark");

      setTheme("light");
      expect(useUIStore.getState().resolvedTheme).toBe("light");
    });
  });

  describe("Color scheme", () => {
    it("sets color-scheme to dark for dark theme", () => {
      useUIStore.getState().setTheme("dark");
      expect(document.documentElement.style.colorScheme).toBe("dark");
    });

    it("sets color-scheme to light for light theme", () => {
      useUIStore.getState().setTheme("light");
      expect(document.documentElement.style.colorScheme).toBe("light");
    });

    it("sets color-scheme to light for high-contrast theme", () => {
      useUIStore.getState().setTheme("high-contrast");
      expect(document.documentElement.style.colorScheme).toBe("light");
    });
  });
});
