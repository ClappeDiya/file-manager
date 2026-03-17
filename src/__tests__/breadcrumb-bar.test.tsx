import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent } from "@testing-library/react";
import { BreadcrumbBar } from "@/components/breadcrumb-bar";

describe("T-009: BreadcrumbBar", () => {
  it("should render breadcrumb navigation", () => {
    render(<BreadcrumbBar path="/home/user/Documents" onNavigate={vi.fn()} />);
    expect(screen.getByTestId("breadcrumb-bar")).toBeInTheDocument();
  });

  it("should show root (home) button", () => {
    render(<BreadcrumbBar path="/home/user" onNavigate={vi.fn()} />);
    expect(screen.getByTestId("breadcrumb-root")).toBeInTheDocument();
  });

  it("should render path segments", () => {
    render(<BreadcrumbBar path="/home/user/Documents" onNavigate={vi.fn()} />);
    expect(screen.getByTestId("breadcrumb-segment-0")).toHaveTextContent("home");
    expect(screen.getByTestId("breadcrumb-segment-1")).toHaveTextContent("user");
    expect(screen.getByTestId("breadcrumb-segment-2")).toHaveTextContent("Documents");
  });

  it("should navigate to root on home click", () => {
    const onNavigate = vi.fn();
    render(<BreadcrumbBar path="/home/user" onNavigate={onNavigate} />);
    fireEvent.click(screen.getByTestId("breadcrumb-root"));
    expect(onNavigate).toHaveBeenCalledWith("/");
  });

  it("should navigate to segment path on click", () => {
    const onNavigate = vi.fn();
    render(<BreadcrumbBar path="/home/user/Documents" onNavigate={onNavigate} />);
    fireEvent.click(screen.getByTestId("breadcrumb-segment-1"));
    expect(onNavigate).toHaveBeenCalledWith("/home/user");
  });

  it("should mark last segment as current page", () => {
    render(<BreadcrumbBar path="/home/user/Documents" onNavigate={vi.fn()} />);
    expect(screen.getByTestId("breadcrumb-segment-2")).toHaveAttribute("aria-current", "page");
    expect(screen.getByTestId("breadcrumb-segment-0")).not.toHaveAttribute("aria-current");
  });

  it("should handle root path", () => {
    render(<BreadcrumbBar path="/" onNavigate={vi.fn()} />);
    expect(screen.getByTestId("breadcrumb-root")).toBeInTheDocument();
    expect(screen.queryByTestId("breadcrumb-segment-0")).not.toBeInTheDocument();
  });

  it("should have accessible navigation role", () => {
    render(<BreadcrumbBar path="/home" onNavigate={vi.fn()} />);
    expect(screen.getByRole("navigation", { name: /breadcrumb/i })).toBeInTheDocument();
  });
});
