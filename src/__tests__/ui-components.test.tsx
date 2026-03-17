import { describe, it, expect } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import {
  Button,
  Input,
  Badge,
  Separator,
  Dialog,
  DialogTrigger,
  DialogContent,
  DialogTitle,
  DialogClose,
  Tabs,
  TabsList,
  TabsTrigger,
  TabsContent,
} from "@ufop/ui-components";

describe("@ufop/ui-components", () => {
  describe("Button", () => {
    it("renders with default variant", () => {
      render(<Button>Click me</Button>);
      expect(screen.getByRole("button", { name: "Click me" })).toBeInTheDocument();
    });

    it("renders all variants", () => {
      const variants = [
        "default",
        "secondary",
        "outline",
        "ghost",
        "destructive",
        "link",
      ] as const;
      variants.forEach((variant) => {
        const { unmount } = render(
          <Button variant={variant}>Test {variant}</Button>,
        );
        expect(
          screen.getByRole("button", { name: `Test ${variant}` }),
        ).toBeInTheDocument();
        unmount();
      });
    });

    it("renders all sizes", () => {
      const sizes = ["sm", "md", "lg", "icon"] as const;
      sizes.forEach((size) => {
        const { unmount } = render(
          <Button size={size} aria-label={`Size ${size}`}>
            {size}
          </Button>,
        );
        expect(screen.getByRole("button")).toBeInTheDocument();
        unmount();
      });
    });

    it("shows loading state", () => {
      render(<Button loading>Loading</Button>);
      const button = screen.getByRole("button");
      expect(button).toHaveAttribute("aria-busy", "true");
      expect(button).toBeDisabled();
    });

    it("shows disabled state", () => {
      render(<Button disabled>Disabled</Button>);
      const button = screen.getByRole("button");
      expect(button).toBeDisabled();
      expect(button).toHaveAttribute("aria-disabled", "true");
    });

    it("calls onClick handler", async () => {
      const user = userEvent.setup();
      let clicked = false;
      render(<Button onClick={() => (clicked = true)}>Click</Button>);
      await user.click(screen.getByRole("button"));
      expect(clicked).toBe(true);
    });

    it("is keyboard accessible", async () => {
      const user = userEvent.setup();
      let clicked = false;
      render(<Button onClick={() => (clicked = true)}>Enter</Button>);
      screen.getByRole("button").focus();
      await user.keyboard("{Enter}");
      expect(clicked).toBe(true);
    });
  });

  describe("Input", () => {
    it("renders a text input", () => {
      render(<Input srLabel="Test input" placeholder="Type here" />);
      expect(screen.getByPlaceholderText("Type here")).toBeInTheDocument();
    });

    it("has screen reader label when srLabel is provided", () => {
      render(<Input srLabel="Email address" />);
      expect(screen.getByLabelText("Email address")).toBeInTheDocument();
    });

    it("shows error state", () => {
      render(<Input error srLabel="Error input" />);
      expect(screen.getByRole("textbox")).toHaveAttribute(
        "aria-invalid",
        "true",
      );
    });

    it("supports disabled state", () => {
      render(<Input disabled srLabel="Disabled input" />);
      expect(screen.getByRole("textbox")).toBeDisabled();
    });
  });

  describe("Badge", () => {
    it("renders with default variant", () => {
      render(<Badge>New</Badge>);
      expect(screen.getByText("New")).toBeInTheDocument();
    });

    it("renders all variants", () => {
      const variants = [
        "default",
        "secondary",
        "success",
        "warning",
        "error",
        "info",
        "outline",
      ] as const;
      variants.forEach((variant) => {
        const { unmount } = render(
          <Badge variant={variant}>{variant}</Badge>,
        );
        expect(screen.getByText(variant)).toBeInTheDocument();
        unmount();
      });
    });
  });

  describe("Separator", () => {
    it("renders as decorative by default", () => {
      const { container } = render(<Separator />);
      const sep = container.firstChild as HTMLElement;
      expect(sep).toHaveAttribute("role", "none");
    });

    it("renders as separator when not decorative", () => {
      const { container } = render(<Separator decorative={false} />);
      const sep = container.firstChild as HTMLElement;
      expect(sep).toHaveAttribute("role", "separator");
    });
  });

  describe("Dialog", () => {
    it("renders trigger and opens dialog", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Test Dialog</DialogTitle>
            <p>Content</p>
            <DialogClose>
              <Button>Close</Button>
            </DialogClose>
          </DialogContent>
        </Dialog>,
      );

      // Dialog should not be visible initially
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();

      // Click trigger to open
      await user.click(screen.getByRole("button", { name: "Open" }));

      // Dialog should be visible
      expect(screen.getByRole("dialog")).toBeInTheDocument();
      expect(screen.getByText("Test Dialog")).toBeInTheDocument();
    });

    it("closes on close button click", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Test Dialog</DialogTitle>
            <DialogClose>
              <Button>Close</Button>
            </DialogClose>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));
      expect(screen.getByRole("dialog")).toBeInTheDocument();

      await user.click(screen.getByRole("button", { name: "Close" }));
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("closes on Escape key", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Test</DialogTitle>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));
      expect(screen.getByRole("dialog")).toBeInTheDocument();

      await user.keyboard("{Escape}");
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    });

    it("has aria-modal attribute", async () => {
      const user = userEvent.setup();
      render(
        <Dialog>
          <DialogTrigger>
            <Button>Open</Button>
          </DialogTrigger>
          <DialogContent>
            <DialogTitle>Modal</DialogTitle>
          </DialogContent>
        </Dialog>,
      );

      await user.click(screen.getByRole("button", { name: "Open" }));
      expect(screen.getByRole("dialog")).toHaveAttribute(
        "aria-modal",
        "true",
      );
    });
  });

  describe("Tabs", () => {
    it("renders tabs with correct ARIA roles", () => {
      render(
        <Tabs defaultValue="tab1">
          <TabsList aria-label="Test tabs">
            <TabsTrigger value="tab1">Tab 1</TabsTrigger>
            <TabsTrigger value="tab2">Tab 2</TabsTrigger>
          </TabsList>
          <TabsContent value="tab1">Content 1</TabsContent>
          <TabsContent value="tab2">Content 2</TabsContent>
        </Tabs>,
      );

      expect(screen.getByRole("tablist")).toBeInTheDocument();
      expect(screen.getAllByRole("tab")).toHaveLength(2);
      expect(screen.getByRole("tabpanel")).toBeInTheDocument();
    });

    it("shows correct content for active tab", () => {
      render(
        <Tabs defaultValue="tab1">
          <TabsList>
            <TabsTrigger value="tab1">Tab 1</TabsTrigger>
            <TabsTrigger value="tab2">Tab 2</TabsTrigger>
          </TabsList>
          <TabsContent value="tab1">Content 1</TabsContent>
          <TabsContent value="tab2">Content 2</TabsContent>
        </Tabs>,
      );

      expect(screen.getByText("Content 1")).toBeInTheDocument();
      expect(screen.queryByText("Content 2")).not.toBeInTheDocument();
    });

    it("switches tabs on click", async () => {
      const user = userEvent.setup();
      render(
        <Tabs defaultValue="tab1">
          <TabsList>
            <TabsTrigger value="tab1">Tab 1</TabsTrigger>
            <TabsTrigger value="tab2">Tab 2</TabsTrigger>
          </TabsList>
          <TabsContent value="tab1">Content 1</TabsContent>
          <TabsContent value="tab2">Content 2</TabsContent>
        </Tabs>,
      );

      await user.click(screen.getByRole("tab", { name: "Tab 2" }));
      expect(screen.queryByText("Content 1")).not.toBeInTheDocument();
      expect(screen.getByText("Content 2")).toBeInTheDocument();
    });

    it("has correct aria-selected on active tab", () => {
      render(
        <Tabs defaultValue="tab1">
          <TabsList>
            <TabsTrigger value="tab1">Tab 1</TabsTrigger>
            <TabsTrigger value="tab2">Tab 2</TabsTrigger>
          </TabsList>
          <TabsContent value="tab1">Content 1</TabsContent>
          <TabsContent value="tab2">Content 2</TabsContent>
        </Tabs>,
      );

      const tab1 = screen.getByRole("tab", { name: "Tab 1" });
      const tab2 = screen.getByRole("tab", { name: "Tab 2" });

      expect(tab1).toHaveAttribute("aria-selected", "true");
      expect(tab2).toHaveAttribute("aria-selected", "false");
    });

    it("supports keyboard navigation with arrow keys", async () => {
      const user = userEvent.setup();
      render(
        <Tabs defaultValue="tab1">
          <TabsList>
            <TabsTrigger value="tab1">Tab 1</TabsTrigger>
            <TabsTrigger value="tab2">Tab 2</TabsTrigger>
          </TabsList>
          <TabsContent value="tab1">Content 1</TabsContent>
          <TabsContent value="tab2">Content 2</TabsContent>
        </Tabs>,
      );

      const tab1 = screen.getByRole("tab", { name: "Tab 1" });
      tab1.focus();
      await user.keyboard("{ArrowRight}");

      // Tab 2 should now be active
      expect(screen.getByText("Content 2")).toBeInTheDocument();
    });
  });
});
