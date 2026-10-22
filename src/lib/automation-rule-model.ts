/**
 * Editor model for automation rules: the form the rule editor edits, and
 * conversion to and from the `AutomationRule` the Rust engine stores.
 *
 * Kept free of React so presets, defaults and the condition-tree mapping
 * are unit-tested on their own. Templates (destinations, names, messages)
 * are rendered by Rust (`naming_preview`), never here.
 */
import type {
  ActionStep,
  AutomationRule,
  ConditionNode,
  OverwritePolicy,
  RuleAction,
  RuleCondition,
  RuleOptions,
  RuleTrigger,
  Workflow,
} from "@/stores/automation-store";
import { formatBytes } from "@/lib/format-bytes";
import { validateWorkflow, workflowActions } from "@/lib/workflow-model";

export interface ConditionRow {
  key: string;
  negate: boolean;
  condition: RuleCondition;
}

export interface RuleForm {
  id?: string;
  name: string;
  enabled: boolean;
  trigger: RuleTrigger;
  match: "all" | "any";
  conditions: ConditionRow[];
  steps: ActionStep[];
  options: RuleOptions;
  /** A stored tree too complex for the row editor; kept untouched. */
  customTree: ConditionNode | null;
  /** The rule's actions as a graph (replaces `steps` when set). */
  workflow?: Workflow | null;
  /** Fields of the original rule the editor doesn't show. */
  base?: AutomationRule;
}

let keySeq = 0;
export const rowKey = () => `c${++keySeq}`;

export const NO_RETRY = { retries: 0, delay_ms: 0, exponential: false, max_delay_ms: 0 };
export const DEFAULT_RETRY = { retries: 3, delay_ms: 5_000, exponential: true, max_delay_ms: 300_000 };

/** Options for a rule created in the editor. Stored rules keep theirs. */
export function defaultOptions(): RuleOptions {
  return {
    overwrite: "rename",
    retry: { ...DEFAULT_RETRY },
    priority: 0,
    allow_overlap: false,
    settle_secs: 3,
    source: null,
  };
}

export const TRIGGER_TYPES: { type: RuleTrigger["type"]; label: string }[] = [
  { type: "file_appears", label: "A file arrives in a folder" },
  { type: "file_modified", label: "A file changes in a folder" },
  { type: "file_deleted", label: "A file is deleted from a folder" },
  { type: "schedule", label: "On a schedule" },
  { type: "manual", label: "Only when I run it" },
  { type: "webhook", label: "When a webhook URL is called" },
];

export const CRON_PRESETS: { label: string; cron: string }[] = [
  { label: "Every 15 minutes", cron: "0 */15 * * * * *" },
  { label: "Every hour", cron: "0 0 * * * * *" },
  { label: "Every day at 02:00", cron: "0 0 2 * * * *" },
  { label: "Weekdays at 09:00", cron: "0 0 9 * * Mon-Fri *" },
  { label: "Every Sunday at 03:00", cron: "0 0 3 * * Sun *" },
  { label: "First day of the month", cron: "0 0 1 1 * * *" },
];

export function defaultTrigger(type: RuleTrigger["type"], folder = ""): RuleTrigger {
  switch (type) {
    case "file_appears":
    case "file_modified":
    case "file_deleted":
      return { type, watch_path: folder, recursive: false };
    case "schedule":
      return { type, cron_expr: CRON_PRESETS[2].cron, cron_human: CRON_PRESETS[2].label };
    case "manual":
      return { type: "manual" };
    case "webhook":
      return { type: "webhook" };
  }
}

export const isFileTrigger = (t: RuleTrigger) =>
  t.type === "file_appears" || t.type === "file_modified" || t.type === "file_deleted";

export const CONDITION_TYPES: { type: RuleCondition["type"]; label: string }[] = [
  { type: "extension_is", label: "Extension is" },
  { type: "name_matches", label: "Name matches (glob)" },
  { type: "name_regex", label: "Name matches (regex)" },
  { type: "path_contains", label: "Path contains" },
  { type: "size_greater_than", label: "Larger than (bytes)" },
  { type: "size_less_than", label: "Smaller than (bytes)" },
  { type: "older_than", label: "Not modified for (seconds)" },
  { type: "modified_within", label: "Modified within (seconds)" },
  { type: "created_within", label: "Created within (seconds)" },
];

export function defaultCondition(type: RuleCondition["type"]): RuleCondition {
  switch (type) {
    case "extension_is":
      return { type, extensions: ["pdf"] };
    case "name_matches":
      return { type, pattern: "*" };
    case "name_regex":
      return { type, pattern: "^(.*)$" };
    case "path_contains":
      return { type, substring: "" };
    case "size_greater_than":
    case "size_less_than":
      return { type, bytes: 1_048_576 };
    case "older_than":
      return { type, seconds: 30 * 86_400 };
    case "modified_within":
    case "created_within":
      return { type, seconds: 3600 };
  }
}

export const ACTION_TYPES: { type: RuleAction["type"]; label: string; advanced?: boolean }[] = [
  { type: "move_file", label: "Move to folder" },
  { type: "copy_file", label: "Copy to folder" },
  { type: "rename_file", label: "Rename" },
  { type: "delete_file", label: "Delete" },
  { type: "compress_folder", label: "Compress" },
  { type: "extract_archive", label: "Extract archive" },
  { type: "create_folder", label: "Create folder" },
  { type: "transfer_to_connector", label: "Upload to a connection" },
  { type: "run_sync", label: "Run a sync pair" },
  { type: "notify", label: "Show a notification" },
  { type: "send_message", label: "Send email / Slack / Teams" },
  { type: "execute_custom_command", label: "Run a saved custom command", advanced: true },
  { type: "run_command", label: "Run a program", advanced: true },
  { type: "download_url", label: "Download a URL", advanced: true },
  { type: "convert_image", label: "Resize / convert an image" },
  { type: "convert_document", label: "Convert a document (LibreOffice)" },
  { type: "validate_batch", label: "Validate a batch of files", advanced: true },
  { type: "ai_task", label: "Ask the local AI (summarize, classify…)", advanced: true },
  { type: "scan_for_malware", label: "Scan for malware (ClamAV)" },
];

export function defaultAction(type: RuleAction["type"]): RuleAction {
  switch (type) {
    case "move_file":
    case "copy_file":
      return { type, dest_path: "" };
    case "rename_file":
      return { type, pattern: "{yyyy}-{mm}-{dd}_{name}.{ext}" };
    case "delete_file":
      return { type, to_trash: true };
    case "compress_folder":
      return { type, dest_path: "", format: "zip" };
    case "extract_archive":
      return { type, dest_path: "", max_entries: 10_000, delete_archive: false };
    case "create_folder":
      return { type, path: "" };
    case "transfer_to_connector":
      return { type, connector_protocol: "sftp", connection_id: "", remote_path: "/" };
    case "run_sync":
      return { type, sync_pair_id: "" };
    case "notify":
      return { type, title: "{rule}", body: "{filename}" };
    case "send_message":
      return { type, notification: { type: "slack", credential_id: crypto.randomUUID() }, title: "{rule}", body: "{filename} arrived" };
    case "execute_custom_command":
      return { type, command_id: "" };
    case "run_command":
      return { type, program: "", args: ["{path}"], working_dir: null, timeout_secs: 300 };
    case "replay_op":
      return { type, kind: "copy", source_paths: [], dest_path: "" };
    case "download_url":
      return { type, url: "https://", dest_path: "", file_name: null, overwrite: false, max_mb: 1024, sha256: null, allow_private: false };
    case "convert_image":
      return { type, format: "jpeg", max_width: 1920, max_height: 1920, quality: 85, dest_path: "" };
    case "convert_document":
      return { type, to: "pdf", dest_path: "" };
    case "scan_for_malware":
      return { type, quarantine_path: "~/Quarantine", notify: true };
    case "ai_task":
      return { type, instruction: "Summarize this file in three bullet points.", dest_path: "", suffix: "ai.md", max_chars: 20_000 };
    case "validate_batch":
      return { type, folder: "", rules: { members: [{ pattern: "*.csv", min: 1, max: null, min_bytes: null }], content: [], no_extra_files: false }, report_folder: null, fail_on_invalid: true };
  }
}

export const OVERWRITE_LABELS: Record<OverwritePolicy, string> = {
  rename: "Keep both (add a number)",
  skip_always: "Skip the file",
  overwrite_always: "Replace",
  overwrite_if_newer: "Replace if the new file is newer",
  overwrite_if_larger: "Replace if the new file is larger",
  ask_each_time: "Keep both (add a number)",
};

export function newRuleForm(folder = ""): RuleForm {
  return {
    name: "New rule",
    enabled: false,
    trigger: defaultTrigger("file_appears", folder),
    match: "all",
    conditions: [],
    steps: [{ action: defaultAction("move_file"), continue_on_error: false }],
    options: defaultOptions(),
    customTree: null,
    workflow: null,
  };
}

/** Rows for a tree the row editor can show: all/any of leaves or not(leaf). */
function rowsFromTree(tree: ConditionNode): { match: "all" | "any"; rows: ConditionRow[] } | null {
  if (tree.type !== "all" && tree.type !== "any") return null;
  const rows: ConditionRow[] = [];
  for (const n of tree.nodes) {
    if (n.type === "condition") rows.push({ key: rowKey(), negate: false, condition: n.condition });
    else if (n.type === "not" && n.node.type === "condition") rows.push({ key: rowKey(), negate: true, condition: n.node.condition });
    else return null;
  }
  return { match: tree.type, rows };
}

export function formFromRule(rule: AutomationRule): RuleForm {
  let match: "all" | "any" = "all";
  let conditions: ConditionRow[] = rule.conditions.map((c) => ({ key: rowKey(), negate: false, condition: c }));
  let customTree: ConditionNode | null = null;
  if (rule.condition_tree) {
    const simple = rowsFromTree(rule.condition_tree);
    if (simple) {
      match = simple.match;
      conditions = simple.rows;
    } else {
      customTree = rule.condition_tree;
      conditions = [];
    }
  }
  return {
    id: rule.id,
    name: rule.name,
    enabled: rule.enabled,
    trigger: rule.trigger,
    match,
    conditions,
    steps: [{ action: rule.action, continue_on_error: false }, ...(rule.steps ?? [])],
    // Stored rules keep their own options (absent = engine defaults).
    options: rule.options ?? { overwrite: "rename", retry: { ...NO_RETRY }, priority: 0, allow_overlap: false, settle_secs: 0, source: null },
    customTree,
    workflow: rule.workflow ?? null,
    base: rule,
  };
}

export function ruleFromForm(form: RuleForm): AutomationRule {
  const now = new Date().toISOString();
  const base = form.base;
  const leaves = form.conditions.map((r) => r.condition);
  const needsTree = form.match === "any" || form.conditions.some((r) => r.negate);
  const tree: ConditionNode | null = form.customTree
    ? form.customTree
    : needsTree
      ? {
          type: form.match,
          nodes: form.conditions.map((r) =>
            r.negate
              ? { type: "not", node: { type: "condition", condition: r.condition } }
              : { type: "condition", condition: r.condition },
          ),
        }
      : null;
  // With a workflow, `action` is its first action (the engine runs the graph).
  const firstNodeAction = form.workflow ? workflowActions(form.workflow)[0] : undefined;
  const [first, ...rest] = form.workflow ? [{ action: firstNodeAction ?? form.steps[0].action, continue_on_error: false }] : form.steps;
  const source = isFileTrigger(form.trigger) ? null : form.options.source ?? null;
  return {
    id: form.id ?? crypto.randomUUID(),
    name: form.name.trim(),
    enabled: form.enabled,
    trigger: form.trigger,
    conditions: tree ? [] : leaves,
    condition_tree: tree,
    action: first.action,
    steps: rest,
    options: { ...form.options, source },
    created_at: base?.created_at ?? now,
    updated_at: now,
    last_triggered: base?.last_triggered ?? null,
    trigger_count: base?.trigger_count ?? 0,
    error_count: base?.error_count ?? 0,
    last_error: base?.last_error ?? null,
    ...(form.workflow ? { workflow: form.workflow } : {}),
  };
}

/** Problems with one action (`n` names it in the message). */
function actionErrors(a: RuleAction, n: string): string[] {
  const errors: string[] = [];
  if ((a.type === "move_file" || a.type === "copy_file" || a.type === "compress_folder" || a.type === "extract_archive") && !a.dest_path.trim()) errors.push(`${n}: choose a destination.`);
  if (a.type === "create_folder" && !a.path.trim()) errors.push(`${n}: choose the folder to create.`);
  if (a.type === "rename_file" && !a.pattern.trim()) errors.push(`${n}: enter a name pattern.`);
  if (a.type === "run_sync" && !a.sync_pair_id) errors.push(`${n}: choose a sync pair.`);
  if (a.type === "transfer_to_connector" && !a.connection_id) errors.push(`${n}: choose a connection.`);
  if (a.type === "run_command" && !a.program.trim()) errors.push(`${n}: enter the program to run.`);
  if (a.type === "execute_custom_command" && !a.command_id) errors.push(`${n}: choose a custom command.`);
  if (a.type === "convert_image") {
    if (!a.format && !a.max_width && !a.max_height) errors.push(`${n}: choose a format or a largest size.`);
    for (const v of [a.max_width, a.max_height]) if (v !== null && !(v >= 1 && v <= 20_000)) errors.push(`${n}: sizes are 1–20000 pixels.`);
    if (a.quality !== null && !(a.quality >= 1 && a.quality <= 100)) errors.push(`${n}: quality is 1–100.`);
  }
  if (a.type === "scan_for_malware" && !a.quarantine_path.trim()) errors.push(`${n}: choose the quarantine folder.`);
  if (a.type === "ai_task") {
    if (!a.instruction.trim()) errors.push(`${n}: write the instruction for the AI.`);
    if (!/^[\w.-]+$/.test(a.suffix.trim().replace(/^\./, ""))) errors.push(`${n}: the answer's file suffix may use letters, digits, '.', '-' and '_'.`);
    if (!(a.max_chars >= 100 && a.max_chars <= 200_000)) errors.push(`${n}: read 100–200000 characters.`);
  }
  if (a.type === "validate_batch") {
    const { members, content, no_extra_files } = a.rules;
    if (members.length === 0 && content.length === 0 && !no_extra_files) errors.push(`${n}: add a member or content rule.`);
    for (const p of [...members.map((m) => m.pattern), ...content.map((c) => c.pattern)]) {
      if (!p.trim() || /[\\/]/.test(p)) errors.push(`${n}: '${p}' must be a file name pattern like *.csv.`);
    }
    for (const m of members) if (m.max !== null && m.max < m.min) errors.push(`${n}: '${m.pattern}' allows fewer files than it requires.`);
  }
  if (a.type === "download_url") {
    if (!/^https?:\/\/[^/\s]+/i.test(a.url.trim())) errors.push(`${n}: enter an http(s) URL to download.`);
    if (!a.dest_path.trim()) errors.push(`${n}: choose the folder to save into.`);
    if (!(a.max_mb >= 1)) errors.push(`${n}: the size limit must be at least 1 MB.`);
    if (a.sha256 && !/^([0-9a-f]{64}|.*\{.*\}.*)$/i.test(a.sha256.trim())) errors.push(`${n}: the SHA-256 must be 64 hex digits.`);
  }
  return errors;
}

/** Problems to fix before saving; empty when the form is valid. */
export function validateForm(form: RuleForm): string[] {
  const errors: string[] = [];
  if (!form.name.trim()) errors.push("Give the rule a name.");
  if (isFileTrigger(form.trigger) && "watch_path" in form.trigger && !form.trigger.watch_path.trim()) {
    errors.push("Choose the folder to watch.");
  }
  if (form.trigger.type === "schedule" && !form.trigger.cron_expr.trim()) errors.push("Choose a schedule.");
  if (!form.workflow && form.steps.length === 0) errors.push("Add at least one action.");
  const fileActions = new Set(["move_file", "copy_file", "rename_file", "delete_file", "compress_folder", "extract_archive", "transfer_to_connector", "convert_image", "convert_document", "ai_task", "scan_for_malware"]);
  const formActions = form.workflow ? workflowActions(form.workflow) : form.steps.map((s) => s.action);
  if (!isFileTrigger(form.trigger) && !form.options.source?.path && formActions.some((a) => fileActions.has(a.type))) {
    errors.push("This trigger has no file: choose which folder's files the rule works on.");
  }
  if (form.workflow) {
    errors.push(...validateWorkflow(form.workflow));
    form.workflow.nodes.forEach((node) => {
      if (node.kind === "action") errors.push(...actionErrors(node.action, `Step ${node.id}`));
    });
  } else {
    form.steps.forEach((s, i) => errors.push(...actionErrors(s.action, `Action ${i + 1}`)));
  }
  for (const r of form.conditions) {
    if (r.condition.type === "name_regex") {
      try {
        new RegExp(r.condition.pattern);
      } catch {
        errors.push(`Invalid regular expression: ${r.condition.pattern}`);
      }
    }
  }
  return errors;
}

/** Whether a rule runs programs that need the user's approval. */
/**
 * A workflow that is a plain chain of actions as a list of steps (a step
 * whose outgoing connection fires "always" continues on error); null when
 * it branches or cannot be written as a list.
 */
export function linearSteps(wf: Workflow): ActionStep[] | null {
  if (wf.nodes.length === 0 || wf.nodes.some((n) => n.kind !== "action")) return null;
  const starts = wf.nodes.filter((n) => !wf.edges.some((e) => e.to === n.id));
  if (starts.length !== 1 || wf.edges.length !== wf.nodes.length - 1) return null;
  const steps: ActionStep[] = [];
  let cur: string | undefined = starts[0].id;
  while (cur) {
    const n = wf.nodes.find((x) => x.id === cur)!;
    const out = wf.edges.filter((e) => e.from === cur);
    if (out.length > 1 || out.some((e) => e.on === "failure") || n.kind !== "action") return null;
    // "Always" out of a step = carry on when it fails (the first action can't).
    const keepGoing = out[0]?.on === "always";
    if (keepGoing && steps.length === 0) return null;
    steps.push({ action: n.action, continue_on_error: keepGoing });
    cur = out[0]?.to;
  }
  return steps.length === wf.nodes.length ? steps : null;
}

/** Every action a rule runs: its workflow's, or `action` + `steps`. */
export function ruleActions(rule: AutomationRule): RuleAction[] {
  return rule.workflow ? workflowActions(rule.workflow) : [rule.action, ...(rule.steps ?? []).map((s) => s.action)];
}

const IDENTITY_AND_STATS = ["id", "name", "enabled", "created_at", "updated_at", "last_triggered", "trigger_count", "error_count", "last_error", "managed_by"] as const;

/** A rule's portable definition ("Copy as JSON", pasted into the admin
 * console as an organization workflow): what it does, without its identity,
 * run statistics, command approval or webhook secret. */
export function portableRule(rule: AutomationRule): Record<string, unknown> {
  const def: Record<string, unknown> = { ...rule };
  for (const k of IDENTITY_AND_STATS) delete def[k];
  if (rule.options) {
    const options: Partial<RuleOptions> = { ...rule.options };
    delete options.command_approval;
    delete options.webhook_secret_hash;
    def.options = options;
  }
  return def;
}

export function runsCommands(rule: AutomationRule): boolean {
  return ruleActions(rule).some((a) => a.type === "run_command");
}

/** The exact command lines a rule runs, for the approval prompt. */
export function commandLines(rule: AutomationRule): string[] {
  return ruleActions(rule)
    .filter((a): a is Extract<RuleAction, { type: "run_command" }> => a.type === "run_command")
    .map((a) => [a.program, ...a.args.map((x) => (/\s/.test(x) ? JSON.stringify(x) : x))].join(" ") + (a.working_dir ? `  (in ${a.working_dir})` : ""));
}

// ── Presets (folder-activity notifications, auto-filing, expiry, unzip) ──

export interface RulePreset {
  id: string;
  label: string;
  description: string;
  build: (folder: string) => RuleForm;
}

export const PRESETS: RulePreset[] = [
  {
    id: "notify-arrivals",
    label: "Tell me when files arrive",
    description: "A notification for every new file in a folder.",
    build: (folder) => ({
      ...newRuleForm(folder),
      name: "Notify on new files",
      steps: [{ action: { type: "notify", title: "New file in {parent}", body: "{filename} ({size} bytes)" }, continue_on_error: false }],
    }),
  },
  {
    id: "file-by-date",
    label: "File arrivals by date",
    description: "Move each new file into Year/Month folders.",
    build: (folder) => ({
      ...newRuleForm(folder),
      name: "File by date",
      steps: [{ action: { type: "move_file", dest_path: `${folder || "~"}/Filed/{yyyy}/{mm}` }, continue_on_error: false }],
    }),
  },
  {
    id: "expire-old",
    label: "Clean up old files",
    description: "Every night, move files untouched for 30 days to the folder's trash.",
    build: (folder) => ({
      ...newRuleForm(folder),
      name: "Expire files older than 30 days",
      trigger: defaultTrigger("schedule"),
      conditions: [{ key: rowKey(), negate: false, condition: { type: "older_than", seconds: 30 * 86_400 } }],
      steps: [{ action: { type: "delete_file", to_trash: true }, continue_on_error: false }],
      options: { ...defaultOptions(), source: { path: folder, recursive: false, newest: null, include_dirs: false } },
    }),
  },
  {
    id: "malware-scan",
    label: "Scan arrivals for malware",
    description: "Scan each new file with ClamAV; infected files go to a quarantine folder and you are notified.",
    build: (folder) => ({
      ...newRuleForm(folder),
      name: "Scan new files for malware",
      steps: [{ action: { type: "scan_for_malware", quarantine_path: folder ? `${folder.replace(/[\\/]+$/, "")}-quarantine` : "~/Quarantine", notify: true }, continue_on_error: false }],
    }),
  },
  {
    id: "unzip-arrivals",
    label: "Unzip arriving archives",
    description: "Extract new .zip files next to them and keep the archive.",
    build: (folder) => ({
      ...newRuleForm(folder),
      name: "Extract arriving zips",
      conditions: [{ key: rowKey(), negate: false, condition: { type: "extension_is", extensions: ["zip"] } }],
      steps: [{ action: { type: "extract_archive", dest_path: `${folder}/{name}`, max_entries: 10_000, delete_archive: false }, continue_on_error: false }],
    }),
  },
];

// ── Display ──

export function conditionLabel(condition: RuleCondition): string {
  switch (condition.type) {
    case "extension_is":
      return `ext = ${condition.extensions.join(", ")}`;
    case "name_matches":
      return `name ~ ${condition.pattern}`;
    case "size_greater_than":
      return `size > ${formatBytes(condition.bytes)}`;
    case "size_less_than":
      return `size < ${formatBytes(condition.bytes)}`;
    case "created_within":
      return `age < ${condition.seconds}s`;
    case "path_contains":
      return `path contains "${condition.substring}"`;
    case "older_than":
      return `age > ${condition.seconds}s`;
    case "modified_within":
      return `modified < ${condition.seconds}s ago`;
    case "name_regex":
      return `name =~ /${condition.pattern}/`;
  }
}

/** Human-readable form of a rule's conditions (flat list or tree). */
export function conditionsSummary(rule: AutomationRule): string {
  const node = (n: ConditionNode, top = false): string => {
    switch (n.type) {
      case "condition":
        return conditionLabel(n.condition);
      case "not":
        return `NOT ${node(n.node)}`;
      case "all":
      case "any": {
        const parts = n.nodes.map((c) => node(c));
        const joined = parts.join(n.type === "all" ? " AND " : " OR ");
        return parts.length > 1 && !top ? `(${joined})` : joined;
      }
    }
  };
  if (rule.condition_tree) return node(rule.condition_tree, true);
  return rule.conditions.map(conditionLabel).join(" AND ");
}
