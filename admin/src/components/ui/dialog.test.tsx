// @vitest-environment jsdom
import "@testing-library/jest-dom/vitest";
import { useState } from "react";
import { render, screen, fireEvent } from "@testing-library/react";
import { expect, it } from "vitest";
import { Dialog, DialogTitle } from "./dialog";
it("preserves text-field focus across each controlled keystroke", () => {
  function Form() {
    const [name, setName] = useState("");
    return (
      <Dialog open onClose={() => {}}>
        <DialogTitle>Create policy</DialogTitle>
        <select aria-label="Domain">
          <option>Files</option>
        </select>
        <input
          aria-label="Policy Name"
          value={name}
          onChange={(event) => setName(event.target.value)}
        />
      </Dialog>
    );
  }
  render(<Form />);
  const input = screen.getByRole("textbox", { name: "Policy Name" });
  input.focus();
  for (const value of ["A", "Au", "Aud", "Audi", "Audit"]) {
    fireEvent.change(input, { target: { value } });
    expect(document.activeElement).toBe(input);
  }
  expect(input).toHaveValue("Audit");
  expect(screen.getByRole("dialog", { name: "Create policy" })).toBeVisible();
});
