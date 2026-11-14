import { expect, it } from "vitest";
import { render, screen } from "@testing-library/react";
import { AppErrorScreen } from "@/components/app-error-screen";
it("gives a recoverable production error without exposing internal paths", () => {
  render(
    <AppErrorScreen
      error={new Error("/Users/private/customer-secret.json could not load")}
    />,
  );
  expect(screen.getByRole("alert")).toHaveTextContent(
    "check your recent operation results",
  );
  expect(
    screen.getByRole("button", { name: "Reload FileManager" }),
  ).toBeVisible();
  expect(screen.queryByText(/customer-secret/)).not.toBeInTheDocument();
});
