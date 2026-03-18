import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  Button,
  Input,
  Separator,
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogTitle,
  DialogClose,
  ScrollArea,
} from "@ufop/ui-components";

describe("Accessibility (T-014)", () => {
  describe("Focus indicators", () => {
    it("buttons have focus-visible outline class", () => {
      render(<Button>Focusable</Button>);
      const button = screen.getByRole("button");
      expect(button.className).toContain("focus-visible:");
    });

    it("inputs have focus-visible outline class", () => {
      render(<Input srLabel="Test" />);
      const input = screen.getByRole("textbox");
      expect(input.className).toContain("focus-visible:");
    });

    it("tab triggers have focus-visible outline class", () => {
      render(
        <Tabs defaultValue="a">
          <TabsList>
            <TabsTrigger value="a">A</TabsTrigger>
          </TabsList>
          <TabsContent value="a">Content</TabsContent>
        </Tabs>,
      );
      const tab = screen.getByRole("tab");
      expect(tab.className).toContain("focus-visible:");
    });
  });

  describe("Screen reader labels", () => {
    it("buttons have accessible names", () => {
      render(<Button aria-label="Delete file">X</Button>);
      expect(
        screen.getByRole("button", { name: "Delete file" }),
      ).toBeInTheDocument();
    });

    it("inputs have sr-only labels when srLabel is provided", () => {
      render(<Input srLabel="Search files" />);
      expect(screen.getByLabelText("Search files")).toBeInTheDocument();
    });

    it("separator has appropriate role", () => {
      const { container } = render(<Separator decorative={false} />);
      const sep = container.firstChild as HTMLElement;
      expect(sep).toHaveAttribute("role", "separator");
      expect(sep).toHaveAttribute("aria-orientation", "horizontal");
    });

    it("dialog has aria-modal", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Accessible Dialog</DialogTitle>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));
      const dialog = screen.getByRole("dialog");
      expect(dialog).toHaveAttribute("aria-modal", "true");
    });

    it("scroll area has region role and label", () => {
      render(
        <ScrollArea aria-label="File list">
          <div>Content</div>
        </ScrollArea>,
      );
      expect(
        screen.getByRole("region", { name: "File list" }),
      ).toBeInTheDocument();
    });
  });

  describe("Keyboard navigation", () => {
    it("buttons are activated with Enter", async () => {
      const user = userEvent.setup();
      let activated = false;
      render(
        <Button onClick={() => (activated = true)}>Activate</Button>,
      );
      screen.getByRole("button").focus();
      await user.keyboard("{Enter}");
      expect(activated).toBe(true);
    });

    it("buttons are activated with Space", async () => {
      const user = userEvent.setup();
      let activated = false;
      render(
        <Button onClick={() => (activated = true)}>Activate</Button>,
      );
      screen.getByRole("button").focus();
      await user.keyboard(" ");
      expect(activated).toBe(true);
    });

    it("tabs support arrow key navigation", async () => {
      const user = userEvent.setup();
      render(
        <Tabs defaultValue="first">
          <TabsList>
            <TabsTrigger value="first">First</TabsTrigger>
            <TabsTrigger value="second">Second</TabsTrigger>
            <TabsTrigger value="third">Third</TabsTrigger>
          </TabsList>
          <TabsContent value="first">First Content</TabsContent>
          <TabsContent value="second">Second Content</TabsContent>
          <TabsContent value="third">Third Content</TabsContent>
        </Tabs>,
      );

      const firstTab = screen.getByRole("tab", { name: "First" });
      firstTab.focus();

      // Arrow right to second tab
      await user.keyboard("{ArrowRight}");
      expect(screen.getByText("Second Content")).toBeInTheDocument();

      // Arrow right to third tab
      await user.keyboard("{ArrowRight}");
      expect(screen.getByText("Third Content")).toBeInTheDocument();

      // Arrow right wraps to first tab
      await user.keyboard("{ArrowRight}");
      expect(screen.getByText("First Content")).toBeInTheDocument();
    });

    it("tabs support Home/End keys", async () => {
      const user = userEvent.setup();
      render(
        <Tabs defaultValue="first">
          <TabsList>
            <TabsTrigger value="first">First</TabsTrigger>
            <TabsTrigger value="second">Second</TabsTrigger>
            <TabsTrigger value="third">Third</TabsTrigger>
          </TabsList>
          <TabsContent value="first">First Content</TabsContent>
          <TabsContent value="second">Second Content</TabsContent>
          <TabsContent value="third">Third Content</TabsContent>
        </Tabs>,
      );

      const firstTab = screen.getByRole("tab", { name: "First" });
      firstTab.focus();

      await user.keyboard("{End}");
      expect(screen.getByText("Third Content")).toBeInTheDocument();

      await user.keyboard("{Home}");
      expect(screen.getByText("First Content")).toBeInTheDocument();
    });

    it("dialog traps focus", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Focus Trap</DialogTitle>
            <Button>First</Button>
            <Button>Second</Button>
            <DialogClose>
              <Button>Close</Button>
            </DialogClose>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));

      // Dialog should be open
      expect(screen.getByRole("dialog")).toBeInTheDocument();

      // Focus should be inside the dialog
      const dialog = screen.getByRole("dialog");
      expect(dialog.contains(document.activeElement)).toBe(true);
    });

    it("dialog closes on Escape", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Escape Test</DialogTitle>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));
      expect(screen.getByRole("dialog")).toBeInTheDocument();

      await user.keyboard("{Escape}");
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });
  });

  describe("ARIA states", () => {
    it("disabled button has aria-disabled", () => {
      render(<Button disabled>Disabled</Button>);
      expect(screen.getByRole("button")).toHaveAttribute(
        "aria-disabled",
        "true",
      );
    });

    it("loading button has aria-busy", () => {
      render(<Button loading>Loading</Button>);
      expect(screen.getByRole("button")).toHaveAttribute(
        "aria-busy",
        "true",
      );
    });

    it("error input has aria-invalid", () => {
      render(<Input error srLabel="Error field" />);
      expect(screen.getByRole("textbox")).toHaveAttribute(
        "aria-invalid",
        "true",
      );
    });

    it("selected tab has aria-selected true", () => {
      render(
        <Tabs defaultValue="active">
          <TabsList>
            <TabsTrigger value="active">Active</TabsTrigger>
            <TabsTrigger value="inactive">Inactive</TabsTrigger>
          </TabsList>
          <TabsContent value="active">Active Content</TabsContent>
          <TabsContent value="inactive">Inactive Content</TabsContent>
        </Tabs>,
      );

      expect(
        screen.getByRole("tab", { name: "Active" }),
      ).toHaveAttribute("aria-selected", "true");
      expect(
        screen.getByRole("tab", { name: "Inactive" }),
      ).toHaveAttribute("aria-selected", "false");
    });

    it("tab panel has aria-labelledby linking to tab", () => {
      render(
        <Tabs defaultValue="test">
          <TabsList>
            <TabsTrigger value="test">Test Tab</TabsTrigger>
          </TabsList>
          <TabsContent value="test">Panel Content</TabsContent>
        </Tabs>,
      );

      const panel = screen.getByRole("tabpanel");
      const tab = screen.getByRole("tab");
      expect(panel).toHaveAttribute("aria-labelledby", tab.id);
    });

    it("tab has aria-controls linking to panel", () => {
      render(
        <Tabs defaultValue="test">
          <TabsList>
            <TabsTrigger value="test">Test Tab</TabsTrigger>
          </TabsList>
          <TabsContent value="test">Panel Content</TabsContent>
        </Tabs>,
      );

      const tab = screen.getByRole("tab");
      const panel = screen.getByRole("tabpanel");
      expect(tab).toHaveAttribute("aria-controls", panel.id);
    });
  });
});
