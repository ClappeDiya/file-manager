/**
 * Rule editor model: form <-> rule conversion, the condition tree mapping,
 * validation, command approval lines, presets, and the JSON contract with
 * the Rust engine (the fixture is deserialized by a Rust test too).
 */
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import {
  PRESETS,
  commandLines,
  conditionsSummary,
  defaultAction,
  formFromRule,
  newRuleForm,
  portableRule,
  rowKey,
  ruleFromForm,
  runsCommands,
  validateForm,
  ACTION_TYPES,
  type RuleForm,
} from "@/lib/automation-rule-model";
import type { AutomationRule } from "@/stores/automation-store";

const FIXTURE = resolve(__dirname, "../../src-tauri/src/automation_engine/fixtures/editor-rules.json");

/** Stable fields only: ids and timestamps differ per run. */
function normalize(rule: AutomationRule, i: number): AutomationRule {
  return { ...rule, id: `00000000-0000-4000-8000-${String(i).padStart(12, "0")}`, created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z" };
}

function form(patch: Partial<RuleForm> = {}): RuleForm {
  return { ...newRuleForm("/in"), name: "R", ...patch };
}

describe("ruleFromForm / formFromRule", () => {
  it("keeps a plain AND list flat", () => {
    const rule = ruleFromForm(form({ conditions: [{ key: rowKey(), negate: false, condition: { type: "extension_is", extensions: ["pdf"] } }] }));
    expect(rule.condition_tree).toBeNull();
    expect(rule.conditions).toEqual([{ type: "extension_is", extensions: ["pdf"] }]);
  });

  it("builds a tree for OR and negated rows, and reads it back as rows", () => {
    const f = form({
      match: "any",
      conditions: [
        { key: rowKey(), negate: false, condition: { type: "extension_is", extensions: ["pdf"] } },
        { key: rowKey(), negate: true, condition: { type: "path_contains", substring: "tmp" } },
      ],
    });
    const rule = ruleFromForm(f);
    expect(rule.conditions).toEqual([]);
    expect(rule.condition_tree).toEqual({
      type: "any",
      nodes: [
        { type: "condition", condition: { type: "extension_is", extensions: ["pdf"] } },
        { type: "not", node: { type: "condition", condition: { type: "path_contains", substring: "tmp" } } },
      ],
    });
    const back = formFromRule(rule);
    expect(back.match).toBe("any");
    expect(back.conditions.map((r) => [r.negate, r.condition.type])).toEqual([[false, "extension_is"], [true, "path_contains"]]);
    expect(back.customTree).toBeNull();
  });

  it("preserves a nested tree the row editor cannot show", () => {
    const tree = { type: "all" as const, nodes: [{ type: "any" as const, nodes: [{ type: "condition" as const, condition: { type: "extension_is" as const, extensions: ["a"] } }] }] };
    const rule = { ...ruleFromForm(form()), conditions: [], condition_tree: tree };
    const f = formFromRule(rule);
    expect(f.customTree).toEqual(tree);
    expect(ruleFromForm(f).condition_tree).toEqual(tree);
  });

  it("splits steps into the primary action and follow-ups, keeping counters", () => {
    const base = { ...ruleFromForm(form()), trigger_count: 7, error_count: 2, last_error: "x" };
    const f = formFromRule(base);
    f.steps.push({ action: defaultAction("notify"), continue_on_error: true });
    const rule = ruleFromForm(f);
    expect(rule.action.type).toBe("move_file");
    expect(rule.steps).toEqual([{ action: { type: "notify", title: "{rule}", body: "{filename}" }, continue_on_error: true }]);
    expect([rule.id, rule.trigger_count, rule.error_count, rule.created_at]).toEqual([base.id, 7, 2, base.created_at]);
  });

  it("drops the source selection for file triggers", () => {
    const rule = ruleFromForm(form({ options: { ...newRuleForm().options, source: { path: "/x", recursive: false, newest: null, include_dirs: false } } }));
    expect(rule.options?.source).toBeNull();
  });
});

describe("validateForm", () => {
  it("accepts a complete rule", () => {
    expect(validateForm(form({ steps: [{ action: { type: "move_file", dest_path: "/out" }, continue_on_error: false }] }))).toEqual([]);
  });

  it("reports each missing piece", () => {
    const errors = validateForm({
      ...form({ name: " " }),
      trigger: { type: "file_appears", watch_path: "", recursive: false },
      conditions: [{ key: rowKey(), negate: false, condition: { type: "name_regex", pattern: "(" } }],
      steps: [
        { action: { type: "move_file", dest_path: "" }, continue_on_error: false },
        { action: { type: "run_command", program: "", args: [], working_dir: null, timeout_secs: 5 }, continue_on_error: false },
      ],
    });
    expect(errors).toEqual(expect.arrayContaining([
      "Give the rule a name.",
      "Choose the folder to watch.",
      "Action 1: choose a destination.",
      "Action 2: enter the program to run.",
      "Invalid regular expression: (",
    ]));
  });

  it("asks for a source folder when a scheduled rule acts on files", () => {
    const f = form({ trigger: { type: "schedule", cron_expr: "0 3 * * *", cron_human: "" }, steps: [{ action: { type: "delete_file", to_trash: true }, continue_on_error: false }] });
    expect(validateForm(f)).toContain("This trigger has no file: choose which folder's files the rule works on.");
  });
});

describe("commands and summaries", () => {
  it("lists the exact command lines for approval", () => {
    const rule = ruleFromForm(form({ steps: [
      { action: { type: "notify", title: "t", body: "b" }, continue_on_error: false },
      { action: { type: "run_command", program: "convert", args: ["{path}", "out file.png"], working_dir: "/w", timeout_secs: 9 }, continue_on_error: false },
    ] }));
    expect(runsCommands(rule)).toBe(true);
    expect(commandLines(rule)).toEqual(['convert {path} "out file.png"  (in /w)']);
    expect(runsCommands(ruleFromForm(form()))).toBe(false);
  });

  it("summarizes flat lists and trees without unbalanced parentheses", () => {
    const leaf = (e: string) => ({ type: "condition" as const, condition: { type: "extension_is" as const, extensions: [e] } });
    const base = ruleFromForm(form());
    expect(conditionsSummary({ ...base, conditions: [{ type: "older_than", seconds: 60 }, { type: "name_regex", pattern: "^a" }] })).toBe("age > 60s AND name =~ /^a/");
    const tree = { type: "any" as const, nodes: [{ type: "all" as const, nodes: [leaf("a"), leaf("b")] }, { type: "all" as const, nodes: [leaf("c"), { type: "not" as const, node: leaf("d") }] }] };
    expect(conditionsSummary({ ...base, condition_tree: tree })).toBe("(ext = a AND ext = b) OR (ext = c AND NOT ext = d)");
  });

  it("exports a portable definition without identity, stats, approvals or secrets", () => {
    const rule: AutomationRule = { ...ruleFromForm(form()), trigger_count: 4, last_error: "x", managed_by: "Acme" };
    rule.options = { ...rule.options!, command_approval: "fp", webhook_secret_hash: "h" };
    const def = portableRule(rule);
    for (const k of ["id", "name", "enabled", "created_at", "updated_at", "last_triggered", "trigger_count", "error_count", "last_error", "managed_by"]) expect(def).not.toHaveProperty(k);
    expect(def).toMatchObject({ trigger: rule.trigger, action: rule.action, conditions: rule.conditions });
    expect(def.options).not.toHaveProperty("command_approval");
    expect(def.options).not.toHaveProperty("webhook_secret_hash");
    expect(rule.options!.command_approval).toBe("fp");
  });

  it("has a default for every action type", () => {
    for (const a of ACTION_TYPES) expect(defaultAction(a.type).type).toBe(a.type);
  });
});

describe("download_url", () => {
  it("validates URL, folder, limit and checksum", () => {
    const step = (a: Partial<Extract<ReturnType<typeof defaultAction>, { type: "download_url" }>>) =>
      validateForm(form({ trigger: { type: "manual" }, steps: [{ action: { ...(defaultAction("download_url") as Extract<ReturnType<typeof defaultAction>, { type: "download_url" }>), ...a }, continue_on_error: false }] }));
    expect(step({ url: "https://h/x.csv", dest_path: "/out" })).toEqual([]);
    expect(step({ url: "https://h/{name}.csv", dest_path: "/out", sha256: "{sha}" })).toEqual([]);
    expect(step({ url: "ftp://h/x", dest_path: "/out" })).toEqual(["Action 1: enter an http(s) URL to download."]);
    expect(step({ url: "https://h/x", dest_path: " " })).toEqual(["Action 1: choose the folder to save into."]);
    expect(step({ url: "https://h/x", dest_path: "/o", max_mb: 0 })).toEqual(["Action 1: the size limit must be at least 1 MB."]);
    expect(step({ url: "https://h/x", dest_path: "/o", sha256: "abc" })).toEqual(["Action 1: the SHA-256 must be 64 hex digits."]);
  });
});

describe("presets", () => {
  it("each preset builds a valid rule for a folder", () => {
    for (const p of PRESETS) expect([p.id, validateForm(p.build("/inbox"))]).toEqual([p.id, []]);
  });

  it("match the JSON contract the Rust engine deserializes", () => {
    const rules = PRESETS.map((p, i) => normalize(ruleFromForm(p.build("/inbox")), i));
    // A rule exercising every non-preset shape the editor emits.
    rules.push(normalize(ruleFromForm(form({
      trigger: { type: "webhook" },
      match: "any",
      conditions: [
        { key: rowKey(), negate: true, condition: { type: "modified_within", seconds: 30 } },
        { key: rowKey(), negate: false, condition: { type: "name_regex", pattern: "^(?P<y>\\d{4})" } },
      ],
      steps: [
        { action: { type: "send_message", notification: { type: "teams", credential_id: "cred-1" }, title: "{rule}", body: "{customer}" }, continue_on_error: true },
        { action: { type: "run_command", program: "/bin/echo", args: ["{payload}"], working_dir: null, timeout_secs: 30 }, continue_on_error: false },
        { action: { type: "download_url", url: "https://example.com/{customer}.csv", dest_path: "/in/{yyyy}", file_name: null, overwrite: true, max_mb: 50, sha256: null, allow_private: false }, continue_on_error: false },
        { action: { type: "convert_image", format: "webp", max_width: 800, max_height: null, quality: null, dest_path: "/in/small" }, continue_on_error: false },
        { action: { type: "convert_document", to: "pdf", dest_path: "" }, continue_on_error: false },
        {
          action: {
            type: "validate_batch",
            folder: "",
            rules: {
              members: [{ pattern: "orders_*.csv", min: 1, max: 10, min_bytes: 20 }],
              content: [
                { pattern: "*.csv", check: { type: "csv", required_columns: ["id", "total"], min_rows: 1, delimiter: ";" } },
                { pattern: "SHA256SUMS", check: { type: "sha256_manifest" } },
                { pattern: "*.json", check: { type: "json" } },
                { pattern: "*", check: { type: "not_empty" } },
              ],
              no_extra_files: true,
            },
            report_folder: "/reports/{yyyy}",
            fail_on_invalid: true,
          },
          continue_on_error: false,
        },
        { action: { type: "ai_task", instruction: "Classify {filename}", dest_path: "/ai", suffix: "class.txt", max_chars: 5000 }, continue_on_error: true },
        { action: { type: "scan_for_malware", quarantine_path: "/quarantine/{yyyy}", notify: false }, continue_on_error: false },
      ],
      options: { ...newRuleForm().options, source: { path: "/inbox", recursive: true, newest: 5, include_dirs: false }, priority: -1 },
    })), rules.length));
    // A workflow rule: every node kind and edge kind the builder emits.
    rules.push(normalize(ruleFromForm(form({
      trigger: { type: "manual" },
      workflow: {
        nodes: [
          { id: "n1", kind: "condition", condition: { type: "condition", condition: { type: "extension_is", extensions: ["pdf"] } }, label: "PDF?", x: 0, y: 0 },
          { id: "n2", kind: "action", action: { type: "move_file", dest_path: "/pdf" }, label: "", x: -120, y: 120 },
          { id: "n3", kind: "delay", seconds: 5, label: "", x: 120, y: 120 },
          { id: "n4", kind: "join", all: true, label: "", x: 0, y: 240 },
          { id: "n5", kind: "sub_rule", rule_id: "00000000-0000-4000-8000-000000000000", label: "", x: 0, y: 360 },
        ],
        edges: [
          { from: "n1", to: "n2", on: "true" },
          { from: "n1", to: "n3", on: "false" },
          { from: "n2", to: "n4", on: "success" },
          { from: "n3", to: "n4", on: "always" },
          { from: "n4", to: "n5", on: "failure" },
        ],
      },
    })), rules.length));
    const json = JSON.stringify(rules, null, 2) + "\n";
    if (process.env.UPDATE_FIXTURES) writeFileSync(FIXTURE, json);
    expect(json).toBe(readFileSync(FIXTURE, "utf8"));
  });
});
