/** Batch validation rules editor: member rules, content checks and their CSV options. */
import { describe, expect, it } from "vitest";
import { fireEvent, render, screen, within } from "@testing-library/react";
import { useState } from "react";
import { BatchRulesEditor } from "@/components/batch-rules-editor";
import type { BatchRules } from "@/stores/automation-store";

function Harness({ onRules }: { onRules: (r: BatchRules) => void }) {
  const [rules, setRules] = useState<BatchRules>({ members: [], content: [], no_extra_files: false });
  return (
    <BatchRulesEditor
      rules={rules}
      onChange={(r) => {
        setRules(r);
        onRules(r);
      }}
    />
  );
}

describe("BatchRulesEditor", () => {
  it("builds member and content rules in the engine's shape", () => {
    let last: BatchRules | null = null;
    render(<Harness onRules={(r) => (last = r)} />);
    fireEvent.click(screen.getByText("Add member rule"));
    const member = screen.getByRole("group", { name: "Member rule 1" });
    fireEvent.change(within(member).getByLabelText("Name pattern"), { target: { value: "orders_*.csv" } });
    fireEvent.change(within(member).getByLabelText("At most"), { target: { value: "3" } });
    fireEvent.change(within(member).getByLabelText("Smallest size (bytes)"), { target: { value: "100" } });
    fireEvent.click(screen.getByLabelText(/Other files make the batch invalid/));
    fireEvent.click(screen.getByText("Add content check"));
    const check = screen.getByRole("group", { name: "Content check 1" });
    fireEvent.change(within(check).getByLabelText("Files to check"), { target: { value: "*.csv" } });
    fireEvent.change(within(check).getByLabelText("Check"), { target: { value: "csv" } });
    fireEvent.change(within(check).getByLabelText("Required columns"), { target: { value: "id, total," } });
    fireEvent.change(within(check).getByLabelText("Fewest rows"), { target: { value: "1" } });
    expect(last).toEqual({
      members: [{ pattern: "orders_*.csv", min: 1, max: 3, min_bytes: 100 }],
      content: [{ pattern: "*.csv", check: { type: "csv", required_columns: ["id", "total"], min_rows: 1, delimiter: null } }],
      no_extra_files: true,
    });
    fireEvent.click(screen.getByLabelText("Remove member rule 1"));
    expect(last!.members).toEqual([]);
  });
});
