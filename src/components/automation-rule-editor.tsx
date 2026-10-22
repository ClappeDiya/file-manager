/**
 * Automation rule editor (Quickflows).
 *
 * Opens from the automation panel ("+", Edit, a preset, or a parsed
 * natural-language rule). Covers every trigger, AND/OR/NOT conditions,
 * multi-step actions with `{token}` destinations (live preview rendered by
 * Rust), overwrite/retry/priority/settle options, a dry run, command
 * approval and webhook URLs. Rules are saved disabled unless switched on.
 */
import { NamedSchedulePicker } from "@/components/schedule-pickers";
import { WorkflowBuilder } from "@/components/workflow-builder";
import { autoLayout } from "@/lib/workflow-model";
import { refId } from "@/stores/schedule-store";
import { useCallback, useEffect, useState } from "react";
import { ArrowDown, ArrowUp, Building2, ClipboardCopy, FlaskConical, Link2, Plus, ShieldCheck, Trash2, X } from "lucide-react";
import { Button } from "@ufop/ui-components";
import { useFocusTrap } from "@/hooks/use-focus-trap";
import { useUIStore } from "@/stores/ui-store";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { pickFolder } from "@/lib/folder-picker";
import { errorText } from "@/lib/error-text";
import {
  ACTION_TYPES,
  CONDITION_TYPES,
  CRON_PRESETS,
  DEFAULT_RETRY,
  NO_RETRY,
  OVERWRITE_LABELS,
  PRESETS,
  TRIGGER_TYPES,
  commandLines,
  defaultAction,
  defaultCondition,
  defaultTrigger,
  formFromRule,
  isFileTrigger,
  newRuleForm,
  portableRule,
  rowKey,
  ruleFromForm,
  runsCommands,
  validateForm,
  type RuleForm,
  linearSteps,
} from "@/lib/automation-rule-model";
import { BatchRulesEditor } from "@/components/batch-rules-editor";
import {
  DOCUMENT_FORMATS,
  IMAGE_FORMATS,
  useAutomationStore,
  type AutomationLog,
  type AutomationRule,
  type OverwritePolicy,
  type RuleAction,
  type RuleCondition,
  type WebhookInfo,
  type RuleRevision,
  type Workflow,
} from "@/stores/automation-store";

const field = "w-full min-w-0 rounded border border-[var(--color-border)] bg-[var(--color-bg)] px-2 py-1 text-xs";
const label = "block text-[11px] font-medium text-[var(--color-text-muted)] mb-1";

interface Named {
  id: string;
  name: string;
  protocol?: string;
}

function FolderInput({ value, onChange, placeholder, id, ariaLabel }: { value: string; onChange: (v: string) => void; placeholder?: string; id: string; ariaLabel?: string }) {
  return (
    <div className="flex gap-1">
      <input id={id} aria-label={ariaLabel} className={field} value={value} placeholder={placeholder} onChange={(e) => onChange(e.target.value)} />
      <Button
        variant="outline"
        size="sm"
        type="button"
        aria-label={ariaLabel ? `Browse for ${ariaLabel.toLowerCase()}` : undefined}
        onClick={async () => {
          const p = await pickFolder();
          if (p) onChange(p);
        }}
      >
        Browse
      </Button>
    </div>
  );
}

/** Live preview of a template against a sample file (rendered by Rust). */
function TemplatePreview({ template, sample, asPath, regex }: { template: string; sample: string; asPath?: boolean; regex?: string }) {
  const [out, setOut] = useState<{ ok: boolean; text: string } | null>(null);
  useEffect(() => {
    if (!template.includes("{") || !sample) {
      setOut(null);
      return;
    }
    let live = true;
    const t = setTimeout(async () => {
      try {
        const text = await tauriInvoke<string>("naming_preview", { template, samplePath: sample, matchRegex: regex ?? null, asPath: asPath ?? false });
        if (live) setOut({ ok: true, text });
      } catch (e) {
        if (live) setOut({ ok: false, text: errorText(e) });
      }
    }, 250);
    return () => {
      live = false;
      clearTimeout(t);
    };
  }, [template, sample, asPath, regex]);
  if (!out) return null;
  return (
    <p className={`mt-1 text-[11px] font-mono break-all ${out.ok ? "text-[var(--color-text-muted)]" : "text-red-500"}`} data-testid="template-preview">
      {out.ok ? `→ ${out.text}` : out.text}
    </p>
  );
}

function ConditionFields({ c, onChange, id }: { c: RuleCondition; onChange: (c: RuleCondition) => void; id: string }) {
  switch (c.type) {
    case "extension_is":
      return (
        <input
          id={id}
          aria-label="Extensions"
          className={field}
          value={c.extensions.join(", ")}
          placeholder="pdf, docx"
          onChange={(e) => onChange({ ...c, extensions: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) })}
        />
      );
    case "name_matches":
    case "name_regex":
      return <input id={id} aria-label="Pattern" className={`${field} font-mono`} value={c.pattern} onChange={(e) => onChange({ ...c, pattern: e.target.value })} />;
    case "path_contains":
      return <input id={id} aria-label="Text" className={field} value={c.substring} onChange={(e) => onChange({ ...c, substring: e.target.value })} />;
    case "size_greater_than":
    case "size_less_than":
      return <input id={id} aria-label="Bytes" type="number" min={0} className={field} value={c.bytes} onChange={(e) => onChange({ ...c, bytes: Number(e.target.value) })} />;
    case "older_than":
    case "modified_within":
    case "created_within":
      return (
        <div className="flex items-center gap-1">
          <input
            id={id}
            aria-label="Days"
            type="number"
            min={0}
            step="any"
            className={field}
            value={+(c.seconds / 86_400).toFixed(4)}
            onChange={(e) => onChange({ ...c, seconds: Math.round(Number(e.target.value) * 86_400) })}
          />
          <span className="text-[11px] text-[var(--color-text-muted)]">days</span>
        </div>
      );
  }
}

function ActionFields({
  a,
  onChange,
  idx,
  sample,
  regex,
  syncPairs,
  connections,
  customCommands,
  advanced,
}: {
  a: RuleAction;
  onChange: (a: RuleAction) => void;
  idx: number;
  sample: string;
  regex?: string;
  syncPairs: Named[];
  connections: Named[];
  customCommands: Named[];
  advanced: boolean;
}) {
  const id = (k: string) => `step-${idx}-${k}`;
  const tokenHint = <p className="mt-1 text-[11px] text-[var(--color-text-muted)]">Tokens: {"{yyyy} {mm} {dd} {name} {ext} {parent} {1}…"} · filters {"{name|slug}"}</p>;
  switch (a.type) {
    case "move_file":
    case "copy_file":
    case "compress_folder":
    case "extract_archive":
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("dest")}>Destination folder</label>
            <FolderInput id={id("dest")} value={a.dest_path} onChange={(v) => onChange({ ...a, dest_path: v })} placeholder="/Archive/{yyyy}/{mm}" />
            <TemplatePreview template={a.dest_path} sample={sample} asPath regex={regex} />
            {tokenHint}
          </div>
          {a.type === "compress_folder" && (
            <div>
              <label className={label} htmlFor={id("fmt")}>Format</label>
              <select id={id("fmt")} className={field} value={a.format} onChange={(e) => onChange({ ...a, format: e.target.value })}>
                {["zip", "tar", "tar.gz", "7z"].map((f) => (
                  <option key={f}>{f}</option>
                ))}
              </select>
            </div>
          )}
          {a.type === "extract_archive" && (
            <div className="flex gap-3 items-end">
              <div className="w-32">
                <label className={label} htmlFor={id("max")}>Max entries</label>
                <input id={id("max")} type="number" min={1} className={field} value={a.max_entries} onChange={(e) => onChange({ ...a, max_entries: Number(e.target.value) })} />
              </div>
              <label className="flex items-center gap-1 text-xs pb-1">
                <input type="checkbox" checked={a.delete_archive} onChange={(e) => onChange({ ...a, delete_archive: e.target.checked })} />
                Delete the archive afterwards
              </label>
            </div>
          )}
        </div>
      );
    case "rename_file":
      return (
        <div>
          <label className={label} htmlFor={id("pattern")}>New name</label>
          <input id={id("pattern")} className={`${field} font-mono`} value={a.pattern} onChange={(e) => onChange({ ...a, pattern: e.target.value })} />
          <TemplatePreview template={a.pattern} sample={sample} regex={regex} />
          {tokenHint}
        </div>
      );
    case "delete_file":
      return (
        <label className="flex items-center gap-1 text-xs">
          <input type="checkbox" checked={a.to_trash} onChange={(e) => onChange({ ...a, to_trash: e.target.checked })} />
          Move to the folder's .ufop_trash (recoverable) instead of deleting
        </label>
      );
    case "create_folder":
      return (
        <div>
          <label className={label} htmlFor={id("path")}>Folder</label>
          <FolderInput id={id("path")} value={a.path} onChange={(v) => onChange({ ...a, path: v })} />
          <TemplatePreview template={a.path} sample={sample} asPath regex={regex} />
        </div>
      );
    case "run_sync":
      return (
        <div>
          <label className={label} htmlFor={id("pair")}>Sync pair</label>
          <select id={id("pair")} className={field} value={a.sync_pair_id} onChange={(e) => onChange({ ...a, sync_pair_id: e.target.value })}>
            <option value="">Choose…</option>
            {syncPairs.map((p) => (
              <option key={p.id} value={p.id}>{p.name}</option>
            ))}
          </select>
        </div>
      );
    case "transfer_to_connector":
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("conn")}>Connection</label>
            <select
              id={id("conn")}
              className={field}
              value={a.connection_id}
              onChange={(e) => {
                const c = connections.find((x) => x.id === e.target.value);
                onChange({ ...a, connection_id: e.target.value, connector_protocol: c?.protocol ?? a.connector_protocol });
              }}
            >
              <option value="">Choose…</option>
              {connections.map((c) => (
                <option key={c.id} value={c.id}>{c.name}{c.protocol ? ` (${c.protocol})` : ""}</option>
              ))}
            </select>
          </div>
          <div>
            <label className={label} htmlFor={id("remote")}>Remote folder</label>
            <input id={id("remote")} className={field} value={a.remote_path} onChange={(e) => onChange({ ...a, remote_path: e.target.value })} />
            <TemplatePreview template={a.remote_path} sample={sample} regex={regex} />
          </div>
        </div>
      );
    case "notify":
    case "send_message":
      return (
        <div className="space-y-2">
          {a.type === "send_message" && (
            <div>
              <label className={label} htmlFor={id("chan")}>Channel</label>
              <select
                id={id("chan")}
                className={field}
                value={a.notification.type}
                onChange={(e) => {
                  const t = e.target.value as "slack" | "teams" | "email";
                  const credential_id = a.notification.credential_id;
                  onChange({
                    ...a,
                    notification: t === "email" ? { type: "email", credential_id, host: "", port: null, tls: "start_tls", username: "", from: "", to: [] } : { type: t, credential_id },
                  });
                }}
              >
                <option value="slack">Slack</option>
                <option value="teams">Microsoft Teams</option>
                <option value="email">Email</option>
              </select>
              <ChannelSecret credentialId={a.notification.credential_id} kind={a.notification.type} />
              {a.notification.type === "email" && (
                <div className="grid grid-cols-2 gap-2 mt-2">
                  {(["host", "username", "from"] as const).map((k) => (
                    <input
                      key={k}
                      aria-label={`SMTP ${k}`}
                      placeholder={k === "host" ? "smtp.example.com" : k}
                      className={field}
                      value={(a.notification as { [key: string]: unknown })[k] as string}
                      onChange={(e) => onChange({ ...a, notification: { ...a.notification, [k]: e.target.value } as typeof a.notification })}
                    />
                  ))}
                  <input
                    aria-label="Recipients"
                    placeholder="to@example.com, …"
                    className={field}
                    value={a.notification.type === "email" ? a.notification.to.join(", ") : ""}
                    onChange={(e) =>
                      onChange({ ...a, notification: { ...a.notification, to: e.target.value.split(",").map((x) => x.trim()).filter(Boolean) } as typeof a.notification })
                    }
                  />
                </div>
              )}
            </div>
          )}
          <div>
            <label className={label} htmlFor={id("title")}>Title</label>
            <input id={id("title")} className={field} value={a.title} onChange={(e) => onChange({ ...a, title: e.target.value })} />
          </div>
          <div>
            <label className={label} htmlFor={id("body")}>Message</label>
            <textarea id={id("body")} rows={2} className={field} value={a.body} onChange={(e) => onChange({ ...a, body: e.target.value })} />
            <TemplatePreview template={a.body} sample={sample} regex={regex} />
          </div>
        </div>
      );
    case "execute_custom_command":
      return (
        <div>
          <label className={label} htmlFor={id("cmd")}>Custom command</label>
          <select id={id("cmd")} className={field} value={a.command_id} onChange={(e) => onChange({ ...a, command_id: e.target.value })}>
            <option value="">Choose…</option>
            {customCommands.map((c) => (
              <option key={c.id} value={c.id}>{c.name}</option>
            ))}
          </select>
          <p className="mt-1 text-[11px] text-[var(--color-text-muted)]">Only commands on the safe allowlist run unattended.</p>
        </div>
      );
    case "run_command":
      if (!advanced) return <p className="text-xs text-[var(--color-text-muted)]">Switch to Advanced mode to edit programs.</p>;
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("prog")}>Program</label>
            <input id={id("prog")} className={`${field} font-mono`} value={a.program} placeholder="/usr/local/bin/convert" onChange={(e) => onChange({ ...a, program: e.target.value })} />
          </div>
          <div>
            <label className={label} htmlFor={id("args")}>Arguments (one per line, templates allowed)</label>
            <textarea id={id("args")} rows={3} className={`${field} font-mono`} value={a.args.join("\n")} onChange={(e) => onChange({ ...a, args: e.target.value.split("\n") })} />
          </div>
          <div className="w-32">
            <label className={label} htmlFor={id("timeout")}>Timeout (s)</label>
            <input id={id("timeout")} type="number" min={1} className={field} value={a.timeout_secs} onChange={(e) => onChange({ ...a, timeout_secs: Number(e.target.value) })} />
          </div>
          <p className="text-[11px] text-amber-600">Runs without a shell. You approve the exact command line after saving; editing it needs a new approval.</p>
        </div>
      );
    case "replay_op":
      return <p className="text-xs text-[var(--color-text-muted)]">Replays {a.source_paths.length} pinned file(s) to {a.dest_path}.</p>;
    case "download_url":
      if (!advanced) return <p className="text-xs text-[var(--color-text-muted)]">Switch to Advanced mode to edit downloads.</p>;
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("url")}>URL</label>
            <input id={id("url")} className={`${field} font-mono`} value={a.url} placeholder="https://example.com/exports/{yyyy}-{mm}-{dd}.csv" onChange={(e) => onChange({ ...a, url: e.target.value })} />
            <TemplatePreview template={a.url} sample={sample} regex={regex} />
          </div>
          <div>
            <label className={label} htmlFor={id("dest")}>Save into folder</label>
            <FolderInput id={id("dest")} value={a.dest_path} onChange={(v) => onChange({ ...a, dest_path: v })} placeholder="/Downloads/{yyyy}" />
            <TemplatePreview template={a.dest_path} sample={sample} asPath regex={regex} />
            {tokenHint}
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className={label} htmlFor={id("fname")}>File name (optional)</label>
              <input id={id("fname")} className={field} value={a.file_name ?? ""} placeholder="From the server or the URL" onChange={(e) => onChange({ ...a, file_name: e.target.value || null })} />
            </div>
            <div>
              <label className={label} htmlFor={id("maxmb")}>Size limit (MB)</label>
              <input id={id("maxmb")} type="number" min={1} className={field} value={a.max_mb} onChange={(e) => onChange({ ...a, max_mb: Number(e.target.value) })} />
            </div>
          </div>
          <div>
            <label className={label} htmlFor={id("sha")}>Expected SHA-256 (optional)</label>
            <input id={id("sha")} className={`${field} font-mono`} value={a.sha256 ?? ""} onChange={(e) => onChange({ ...a, sha256: e.target.value || null })} />
          </div>
          <label className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={a.overwrite} onChange={(e) => onChange({ ...a, overwrite: e.target.checked })} />
            Replace a file with the same name (otherwise a number is added)
          </label>
          <label className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={a.allow_private} onChange={(e) => onChange({ ...a, allow_private: e.target.checked })} />
            Allow intranet addresses (localhost, private and link-local networks)
          </label>
          <p className="text-[11px] text-[var(--color-text-muted)]">Downloads to a hidden temporary file and moves it into place when complete. Redirects are followed (up to 5); private addresses stay blocked unless allowed above.</p>
        </div>
      );
    case "convert_image": {
      const size = (v: string) => (v.trim() === "" ? null : Number(v));
      return (
        <div className="space-y-2">
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className={label} htmlFor={id("ifmt")}>Format</label>
              <select id={id("ifmt")} className={field} value={a.format ?? ""} onChange={(e) => onChange({ ...a, format: (e.target.value || null) as typeof a.format })}>
                <option value="">Keep the format</option>
                {IMAGE_FORMATS.map((f) => (
                  <option key={f} value={f}>{f.toUpperCase()}</option>
                ))}
              </select>
            </div>
            {a.format === "jpeg" && (
              <div>
                <label className={label} htmlFor={id("q")}>Quality (1–100)</label>
                <input id={id("q")} type="number" min={1} max={100} className={field} value={a.quality ?? ""} onChange={(e) => onChange({ ...a, quality: size(e.target.value) })} />
              </div>
            )}
            <div>
              <label className={label} htmlFor={id("mw")}>Largest width (px)</label>
              <input id={id("mw")} type="number" min={1} className={field} value={a.max_width ?? ""} placeholder="Any" onChange={(e) => onChange({ ...a, max_width: size(e.target.value) })} />
            </div>
            <div>
              <label className={label} htmlFor={id("mh")}>Largest height (px)</label>
              <input id={id("mh")} type="number" min={1} className={field} value={a.max_height ?? ""} placeholder="Any" onChange={(e) => onChange({ ...a, max_height: size(e.target.value) })} />
            </div>
          </div>
          <div>
            <label className={label} htmlFor={id("idest")}>Save into folder (empty = next to the image)</label>
            <FolderInput id={id("idest")} value={a.dest_path} onChange={(v) => onChange({ ...a, dest_path: v })} placeholder="Same folder" />
            {tokenHint}
          </div>
          <p className="text-[11px] text-[var(--color-text-muted)]">Images are shrunk to fit, keeping their proportions, and never enlarged. Later steps work on the converted file; the original stays.</p>
        </div>
      );
    }
    case "ai_task":
      if (!advanced) return <p className="text-xs text-[var(--color-text-muted)]">Switch to Advanced mode to edit AI tasks.</p>;
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("instr")}>Instruction (templates allowed)</label>
            <textarea id={id("instr")} rows={3} className={field} value={a.instruction} onChange={(e) => onChange({ ...a, instruction: e.target.value })} />
          </div>
          <div className="grid grid-cols-2 gap-2">
            <div>
              <label className={label} htmlFor={id("suffix")}>Save the answer as name.</label>
              <input id={id("suffix")} className={`${field} font-mono`} value={a.suffix} onChange={(e) => onChange({ ...a, suffix: e.target.value })} />
            </div>
            <div>
              <label className={label} htmlFor={id("chars")}>Read up to (characters)</label>
              <input id={id("chars")} type="number" min={100} max={200000} className={field} value={a.max_chars} onChange={(e) => onChange({ ...a, max_chars: Number(e.target.value) })} />
            </div>
          </div>
          <div>
            <label className={label} htmlFor={id("aidest")}>Save into folder (empty = next to the file)</label>
            <FolderInput id={id("aidest")} value={a.dest_path} onChange={(v) => onChange({ ...a, dest_path: v })} placeholder="Same folder" />
            {tokenHint}
          </div>
          <p className="text-[11px] text-amber-600">Sends the file&apos;s text (PDFs and office documents included) to your local AI model. Needs AI and “content analysis” allowed in the AI settings; every task is recorded in the AI audit log.</p>
        </div>
      );
    case "validate_batch":
      if (!advanced) return <p className="text-xs text-[var(--color-text-muted)]">Switch to Advanced mode to edit batch validation.</p>;
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("vfolder")}>Batch folder (empty = the folder of the file that started the run)</label>
            <FolderInput id={id("vfolder")} value={a.folder} onChange={(v) => onChange({ ...a, folder: v })} placeholder="Folder of the triggering file" />
          </div>
          <BatchRulesEditor rules={a.rules} onChange={(rules) => onChange({ ...a, rules })} />
          <div>
            <label className={label} htmlFor={id("vreport")}>Write a report into (optional)</label>
            <FolderInput id={id("vreport")} value={a.report_folder ?? ""} onChange={(v) => onChange({ ...a, report_folder: v || null })} placeholder="No report file" />
            {tokenHint}
          </div>
          <label className="flex items-center gap-2 text-xs">
            <input type="checkbox" checked={a.fail_on_invalid} onChange={(e) => onChange({ ...a, fail_on_invalid: e.target.checked })} /> An invalid batch fails this step (later steps, or a workflow&apos;s failure branch, react to it)
          </label>
          <p className="text-[11px] text-[var(--color-text-muted)]">Tip: start the rule when a marker file (such as <code>*.done</code>) arrives, so the whole batch is there.</p>
        </div>
      );
    case "convert_document":
      return (
        <div className="space-y-2">
          <div>
            <label className={label} htmlFor={id("dfmt")}>Convert to</label>
            <select id={id("dfmt")} className={field} value={a.to} onChange={(e) => onChange({ ...a, to: e.target.value as typeof a.to })}>
              {DOCUMENT_FORMATS.map((f) => (
                <option key={f} value={f}>{f.toUpperCase()}</option>
              ))}
            </select>
          </div>
          <div>
            <label className={label} htmlFor={id("ddest")}>Save into folder (empty = next to the document)</label>
            <FolderInput id={id("ddest")} value={a.dest_path} onChange={(v) => onChange({ ...a, dest_path: v })} placeholder="Same folder" />
            {tokenHint}
          </div>
          <p className="text-[11px] text-[var(--color-text-muted)]">Needs LibreOffice installed. Later steps work on the converted file; the original stays.</p>
        </div>
      );
  }
}

/** Store the channel's secret (webhook URL or SMTP password) in the credential store. */
function ChannelSecret({ credentialId, kind }: { credentialId: string; kind: string }) {
  const [value, setValue] = useState("");
  const [saved, setSaved] = useState(false);
  const label = kind === "email" ? "SMTP password" : kind === "teams" ? "Teams webhook URL" : "Slack webhook URL";
  return (
    <div className="flex gap-1 mt-2">
      <input
        aria-label={label}
        type="password"
        className={field}
        placeholder={`${label} (kept in the credential store)`}
        value={value}
        onChange={(e) => {
          setValue(e.target.value);
          setSaved(false);
        }}
      />
      <Button
        variant="outline"
        size="sm"
        type="button"
        disabled={!value}
        onClick={async () => {
            await tauriInvoke("backup_set_destination_secret", { credentialId, secret: value });
          setValue("");
          setSaved(true);
        }}
      >
        {saved ? "Saved" : "Save"}
      </Button>
    </div>
  );
}

export function AutomationRuleEditor() {
  const editorOpen = useAutomationStore((s) => s.editorOpen);
  const editorRule = useAutomationStore((s) => s.editorRule);
  const closeEditor = useAutomationStore((s) => s.closeEditor);
  const createRule = useAutomationStore((s) => s.createRule);
  const updateRule = useAutomationStore((s) => s.updateRule);
  const testRule = useAutomationStore((s) => s.testRule);
  const approveCommand = useAutomationStore((s) => s.approveCommand);
  const rotateWebhook = useAutomationStore((s) => s.rotateWebhook);
  const rules = useAutomationStore((s) => s.rules);
  const advanced = useUIStore((s) => s.appMode === "advanced");

  const [form, setForm] = useState<RuleForm>(() => newRuleForm());
  const [errors, setErrors] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [preview, setPreview] = useState<AutomationLog | null>(null);
  const [saved, setSaved] = useState<AutomationRule | null>(null);
  const [webhook, setWebhook] = useState<WebhookInfo | null>(null);
  const [sample, setSample] = useState("");
  const [syncPairs, setSyncPairs] = useState<Named[]>([]);
  const [connections, setConnections] = useState<Named[]>([]);
  const [customCommands, setCustomCommands] = useState<Named[]>([]);
  const [revisions, setRevisions] = useState<RuleRevision[]>([]);
  const [copied, setCopied] = useState(false);
  const loadRules = useAutomationStore((s) => s.loadRules);

  useEffect(() => {
    if (!editorOpen) return;
    const existing = editorRule?.id ? rules.find((r) => r.id === editorRule.id) : undefined;
    const source = (existing ?? (editorRule as AutomationRule | null)) as AutomationRule | null;
    setForm(source && source.trigger && source.action ? formFromRule(source) : newRuleForm());
    setErrors([]);
    setPreview(null);
    setSaved(existing ?? null);
    setWebhook(null);
    setSample("");
    setCopied(false);
    void tauriInvokeSafe<Named[]>("list_sync_pairs", undefined, []).then(setSyncPairs);
    void tauriInvokeSafe<Named[]>("list_connections", undefined, []).then(setConnections);
    void tauriInvokeSafe<Named[]>("list_custom_commands", undefined, []).then(setCustomCommands);
    setRevisions([]);
    if (existing) void tauriInvokeSafe<RuleRevision[]>("automation_rule_revisions", { ruleId: existing.id }, []).then(setRevisions);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [editorOpen, editorRule]);

  const containerRef = useFocusTrap<HTMLDivElement>({ active: editorOpen, onEscape: closeEditor });
  const update = useCallback((patch: Partial<RuleForm>) => setForm((f) => ({ ...f, ...patch })), []);
  const setStep = (i: number, action: RuleAction) => setForm((f) => ({ ...f, steps: f.steps.map((s, j) => (j === i ? { ...s, action } : s)) }));
  const regex = form.conditions.find((r) => r.condition.type === "name_regex" && !r.negate)?.condition as Extract<RuleCondition, { type: "name_regex" }> | undefined;

  if (!editorOpen) return null;

  const save = async (): Promise<AutomationRule | null> => {
    const problems = validateForm(form);
    setErrors(problems);
    if (problems.length) return null;
    setBusy(true);
    try {
      const rule = ruleFromForm(form);
      const result = form.id ? await updateRule(rule) : await createRule(rule);
      setForm((f) => ({ ...f, id: result.id, base: result }));
      setSaved(result);
      return result;
    } catch (e) {
      setErrors([errorText(e)]);
      return null;
    } finally {
      setBusy(false);
    }
  };

  const dryRun = async () => {
    const problems = validateForm(form);
    setErrors(problems);
    if (problems.length) return;
    const log = await testRule(ruleFromForm(form));
    setPreview(log);
    if (log?.files_affected[0]) setSample(log.files_affected[0]);
  };

  const copyJson = async () => {
    const problems = validateForm(form);
    setErrors(problems);
    if (problems.length) return;
    try {
      await navigator.clipboard.writeText(JSON.stringify(portableRule(ruleFromForm(form)), null, 2));
      setCopied(true);
    } catch (e) {
      setErrors([`Could not copy: ${errorText(e)}`]);
    }
  };

  const fileTrigger = isFileTrigger(form.trigger);
  const managedBy = saved?.managed_by ?? null;
  const needsApproval = saved && runsCommands(saved) && !saved.options?.command_approval;

  return (
    <div ref={containerRef} className="fixed inset-0 z-[60] flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-labelledby="rule-editor-title">
      <div className="absolute inset-0 bg-black/50" aria-hidden="true" onClick={closeEditor} />
      <div className="relative z-10 flex max-h-[90vh] w-full max-w-2xl flex-col rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-secondary)] shadow-2xl">
        <div className="flex items-center justify-between border-b border-[var(--color-border)] px-4 py-3">
          <h2 id="rule-editor-title" className="text-sm font-semibold">{managedBy ? "View rule" : form.id ? "Edit rule" : "New rule"}</h2>
          <div className="flex items-center gap-2">
            {!form.id && (
              <select
                aria-label="Start from a template"
                className={`${field} w-48`}
                value=""
                onChange={(e) => {
                  const p = PRESETS.find((x) => x.id === e.target.value);
                  if (p) setForm(p.build("watch_path" in form.trigger ? form.trigger.watch_path : ""));
                }}
              >
                <option value="">Start from a template…</option>
                {PRESETS.map((p) => (
                  <option key={p.id} value={p.id} title={p.description}>{p.label}</option>
                ))}
              </select>
            )}
            <Button variant="ghost" size="icon" onClick={closeEditor} aria-label="Close rule editor">
              <X className="h-4 w-4" />
            </Button>
          </div>
        </div>

        <div className="flex-1 space-y-4 overflow-y-auto px-4 py-3">
          {managedBy && (
            <p className="flex items-center gap-2 rounded border border-[var(--color-border)] bg-[var(--color-bg)] p-2 text-xs" role="note">
              <Building2 className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
              Managed by {managedBy}. Your organization sets this rule from its admin console; it can't be changed here.
            </p>
          )}
          <fieldset disabled={!!managedBy} className="m-0 min-w-0 space-y-4 border-0 p-0">
          <div className="flex gap-3 items-end">
            <div className="flex-1">
              <label className={label} htmlFor="rule-name">Name</label>
              <input id="rule-name" className={field} value={form.name} onChange={(e) => update({ name: e.target.value })} />
            </div>
            <label className="flex items-center gap-1 text-xs pb-1">
              <input type="checkbox" checked={form.enabled} onChange={(e) => update({ enabled: e.target.checked })} />
              Enabled
            </label>
          </div>

          {/* Trigger */}
          <section aria-labelledby="trigger-h" className="space-y-2">
            <h3 id="trigger-h" className="text-xs font-semibold">When</h3>
            <select
              aria-label="Trigger"
              className={field}
              value={form.trigger.type}
              onChange={(e) => update({ trigger: defaultTrigger(e.target.value as RuleForm["trigger"]["type"], "watch_path" in form.trigger ? form.trigger.watch_path : "") })}
            >
              {TRIGGER_TYPES.filter((t) => advanced || t.type !== "webhook" || form.trigger.type === "webhook").map((t) => (
                <option key={t.type} value={t.type}>{t.label}</option>
              ))}
            </select>
            {"watch_path" in form.trigger && (
              <div className="flex gap-2 items-center">
                <div className="flex-1">
                  <FolderInput id="watch-path" value={form.trigger.watch_path} onChange={(v) => update({ trigger: { ...(form.trigger as { type: "file_appears"; watch_path: string; recursive: boolean }), watch_path: v } })} placeholder="Folder to watch" ariaLabel="Folder to watch" />
                </div>
                <label className="flex items-center gap-1 text-xs">
                  <input type="checkbox" checked={form.trigger.recursive} onChange={(e) => update({ trigger: { ...(form.trigger as { type: "file_appears"; watch_path: string; recursive: boolean }), recursive: e.target.checked } })} />
                  Include subfolders
                </label>
              </div>
            )}
            {form.trigger.type === "schedule" && advanced && (
              <NamedSchedulePicker
                className={field}
                value={form.trigger.cron_expr}
                customCron={refId(form.trigger.cron_expr) ? "0 0 9 * * *" : form.trigger.cron_expr}
                onChange={(cron) => update({ trigger: { type: "schedule", cron_expr: cron, cron_human: null } })}
              />
            )}
            {form.trigger.type === "schedule" && !refId(form.trigger.cron_expr) && (
              <div className="flex gap-2">
                <select
                  aria-label="Schedule preset"
                  className={field}
                  value={CRON_PRESETS.find((p) => p.cron === (form.trigger as { cron_expr: string }).cron_expr)?.cron ?? "custom"}
                  onChange={(e) => {
                    const p = CRON_PRESETS.find((x) => x.cron === e.target.value);
                    if (p) update({ trigger: { type: "schedule", cron_expr: p.cron, cron_human: p.label } });
                  }}
                >
                  {CRON_PRESETS.map((p) => (
                    <option key={p.cron} value={p.cron}>{p.label}</option>
                  ))}
                  <option value="custom">Custom (cron)</option>
                </select>
                <input
                  aria-label="Cron expression"
                  className={`${field} font-mono`}
                  value={form.trigger.cron_expr}
                  onChange={(e) => update({ trigger: { type: "schedule", cron_expr: e.target.value, cron_human: null } })}
                />
              </div>
            )}
            {form.trigger.type === "webhook" && (
              <p className="text-[11px] text-[var(--color-text-muted)]">Save the rule, then create its URL below. JSON fields in the request become tokens, e.g. {"{customer}"}.</p>
            )}
            {!fileTrigger && (
              <div className="rounded border border-dashed border-[var(--color-border)] p-2 space-y-2">
                <p className="text-[11px] text-[var(--color-text-muted)]">Files to work on (leave empty for actions that don't need a file, like Run sync)</p>
                <FolderInput
                  id="source-path"
                  value={form.options.source?.path ?? ""}
                  onChange={(v) => update({ options: { ...form.options, source: v ? { path: v, recursive: form.options.source?.recursive ?? false, newest: form.options.source?.newest ?? null, include_dirs: false } : null } })}
                  placeholder="Source folder"
                  ariaLabel="Source folder"
                />
                {form.options.source && (
                  <div className="flex gap-3 items-center text-xs">
                    <label className="flex items-center gap-1">
                      <input type="checkbox" checked={form.options.source.recursive} onChange={(e) => update({ options: { ...form.options, source: { ...form.options.source!, recursive: e.target.checked } } })} />
                      Include subfolders
                    </label>
                    <label className="flex items-center gap-1">
                      Only the newest
                      <input
                        aria-label="Newest files"
                        type="number"
                        min={1}
                        className={`${field} w-16`}
                        value={form.options.source.newest ?? ""}
                        placeholder="all"
                        onChange={(e) => update({ options: { ...form.options, source: { ...form.options.source!, newest: e.target.value ? Number(e.target.value) : null } } })}
                      />
                      files
                    </label>
                  </div>
                )}
              </div>
            )}
          </section>

          {/* Conditions */}
          <section aria-labelledby="cond-h" className="space-y-2">
            <div className="flex items-center gap-2">
              <h3 id="cond-h" className="text-xs font-semibold">If</h3>
              {form.conditions.length > 1 && (
                <select aria-label="Match" className={`${field} w-auto`} value={form.match} onChange={(e) => update({ match: e.target.value as "all" | "any" })}>
                  <option value="all">all of these match</option>
                  <option value="any">any of these match</option>
                </select>
              )}
            </div>
            {form.customTree && <p className="text-[11px] text-amber-600">This rule uses a nested condition tree that is kept as is.</p>}
            {form.conditions.length === 0 && !form.customTree && <p className="text-[11px] text-[var(--color-text-muted)]">Every file matches.</p>}
            {form.conditions.map((row, i) => (
              <div key={row.key} className="flex gap-1 items-center">
                <select
                  aria-label={`Condition ${i + 1} negation`}
                  className={`${field} w-20`}
                  value={row.negate ? "not" : "is"}
                  onChange={(e) => update({ conditions: form.conditions.map((r) => (r.key === row.key ? { ...r, negate: e.target.value === "not" } : r)) })}
                >
                  <option value="is">is</option>
                  <option value="not">is not</option>
                </select>
                <select
                  aria-label={`Condition ${i + 1} type`}
                  className={`${field} w-48`}
                  value={row.condition.type}
                  onChange={(e) => update({ conditions: form.conditions.map((r) => (r.key === row.key ? { ...r, condition: defaultCondition(e.target.value as RuleCondition["type"]) } : r)) })}
                >
                  {CONDITION_TYPES.map((t) => (
                    <option key={t.type} value={t.type}>{t.label}</option>
                  ))}
                </select>
                <div className="flex-1">
                  <ConditionFields
                    id={`cond-${i}`}
                    c={row.condition}
                    onChange={(c) => update({ conditions: form.conditions.map((r) => (r.key === row.key ? { ...r, condition: c } : r)) })}
                  />
                </div>
                <Button variant="ghost" size="icon" aria-label={`Remove condition ${i + 1}`} onClick={() => update({ conditions: form.conditions.filter((r) => r.key !== row.key) })}>
                  <Trash2 className="h-3.5 w-3.5" />
                </Button>
              </div>
            ))}
            <Button variant="outline" size="sm" onClick={() => update({ conditions: [...form.conditions, { key: rowKey(), negate: false, condition: defaultCondition("extension_is") }] })}>
              <Plus className="h-3.5 w-3.5 mr-1" /> Add condition
            </Button>
          </section>

          {/* Actions */}
          <section aria-labelledby="act-h" className="space-y-2">
            <h3 id="act-h" className="text-xs font-semibold">Then</h3>
            {advanced && !form.workflow && (
              <Button
                variant="outline"
                size="sm"
                onClick={async () => {
                  try {
                    const wf = await tauriInvoke<Workflow>("automation_convert_to_workflow", { rule: ruleFromForm(form) });
                    update({ workflow: autoLayout(wf) });
                  } catch (e) {
                    setErrors([errorText(e)]);
                  }
                }}
              >
                Edit as a workflow (graph)
              </Button>
            )}
            {form.workflow && (
              <>
                <WorkflowBuilder
                  workflow={form.workflow}
                  onChange={(wf) => update({ workflow: wf })}
                  rules={rules.filter((r) => r.id !== form.id).map((r) => ({ id: r.id, name: r.name }))}
                  actionLabel={(a) => ACTION_TYPES.find((t) => t.type === a.type)?.label ?? a.type}
                  renderAction={(a, onChange, idx) => (
                    <ActionFields a={a} onChange={onChange} idx={idx} sample={sample} regex={regex?.pattern} syncPairs={syncPairs} connections={connections} customCommands={customCommands} advanced={advanced} />
                  )}
                  renderCondition={(c, onChange) =>
                    c.type === "condition" ? (
                      <ConditionFields c={c.condition} onChange={(condition) => onChange({ type: "condition", condition })} id="wf-cond" />
                    ) : (
                      <p className="text-[11px] text-[var(--color-text-muted)]">A combined condition; replace it with a single one to edit it here.</p>
                    )
                  }
                  newAction={() => defaultAction("notify")}
                  newCondition={() => ({ type: "condition", condition: defaultCondition("extension_is") })}
                />
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => {
                    const steps = linearSteps(form.workflow!);
                    if (steps) update({ workflow: null, steps });
                    else if (confirm("This workflow has branches; turning it back into a list keeps the previous list of actions and drops the graph. Continue?")) update({ workflow: null });
                  }}
                >
                  Use a list of actions instead
                </Button>
              </>
            )}
            {!form.workflow && form.steps.map((step, i) => (
              <div key={i} className="rounded border border-[var(--color-border)] p-2 space-y-2" data-testid={`step-${i}`}>
                <div className="flex gap-1 items-center">
                  <span className="text-[11px] text-[var(--color-text-muted)] w-5">{i + 1}.</span>
                  <select aria-label={`Action ${i + 1}`} className={field} value={step.action.type} onChange={(e) => setStep(i, defaultAction(e.target.value as RuleAction["type"]))}>
                    {ACTION_TYPES.filter((t) => advanced || !t.advanced || t.type === step.action.type).map((t) => (
                      <option key={t.type} value={t.type}>{t.label}</option>
                    ))}
                    {step.action.type === "replay_op" && <option value="replay_op">Replay pinned operation</option>}
                  </select>
                  <Button variant="ghost" size="icon" aria-label={`Move action ${i + 1} up`} disabled={i === 0} onClick={() => setForm((f) => { const s = [...f.steps]; [s[i - 1], s[i]] = [s[i], s[i - 1]]; return { ...f, steps: s }; })}>
                    <ArrowUp className="h-3.5 w-3.5" />
                  </Button>
                  <Button variant="ghost" size="icon" aria-label={`Move action ${i + 1} down`} disabled={i === form.steps.length - 1} onClick={() => setForm((f) => { const s = [...f.steps]; [s[i + 1], s[i]] = [s[i], s[i + 1]]; return { ...f, steps: s }; })}>
                    <ArrowDown className="h-3.5 w-3.5" />
                  </Button>
                  <Button variant="ghost" size="icon" aria-label={`Remove action ${i + 1}`} disabled={form.steps.length === 1} onClick={() => update({ steps: form.steps.filter((_, j) => j !== i) })}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                </div>
                <ActionFields a={step.action} onChange={(a) => setStep(i, a)} idx={i} sample={sample} regex={regex?.pattern} syncPairs={syncPairs} connections={connections} customCommands={customCommands} advanced={advanced} />
                {i > 0 && (
                  <label className="flex items-center gap-1 text-[11px]">
                    <input type="checkbox" checked={step.continue_on_error} onChange={(e) => update({ steps: form.steps.map((s, j) => (j === i ? { ...s, continue_on_error: e.target.checked } : s)) })} />
                    If this fails, continue with the next action
                  </label>
                )}
              </div>
            ))}
            {!form.workflow && <div className="flex gap-2">
              <Button variant="outline" size="sm" onClick={() => update({ steps: [...form.steps, { action: defaultAction("copy_file"), continue_on_error: false }] })}>
                <Plus className="h-3.5 w-3.5 mr-1" /> Add another destination
              </Button>
              <Button variant="outline" size="sm" onClick={() => update({ steps: [...form.steps, { action: defaultAction("notify"), continue_on_error: true }] })}>
                <Plus className="h-3.5 w-3.5 mr-1" /> Add action
              </Button>
            </div>}
          </section>

          {revisions.length > 0 && form.id && (
            <details className="rounded border border-[var(--color-border)] p-2" data-testid="rule-history">
              <summary className="cursor-pointer text-xs font-semibold">History ({revisions.length} earlier version{revisions.length === 1 ? "" : "s"})</summary>
              <ul className="mt-2 space-y-1 text-xs">
                {revisions.map((r) => (
                  <li key={r.revision} className="flex items-center gap-2">
                    <span>#{r.revision}</span>
                    <span>{new Date(r.saved_at).toLocaleString()}</span>
                    <span className="truncate">{r.name}</span>
                    <span className="text-[var(--color-text-muted)]">{r.workflow ? "workflow" : `${r.actions} action(s)`}</span>
                    <Button
                      variant="outline"
                      size="sm"
                      className="ml-auto"
                      onClick={async () => {
                        try {
                          const restored = await tauriInvoke<AutomationRule>("automation_restore_revision", { ruleId: form.id, revision: r.revision });
                          await loadRules();
                          setForm(formFromRule(restored));
                          setSaved(restored);
                          setRevisions(await tauriInvokeSafe<RuleRevision[]>("automation_rule_revisions", { ruleId: restored.id }, []));
                        } catch (e) {
                          setErrors([errorText(e)]);
                        }
                      }}
                    >
                      Restore
                    </Button>
                  </li>
                ))}
              </ul>
            </details>
          )}

          {/* Options */}
          <details className="rounded border border-[var(--color-border)] p-2">
            <summary className="cursor-pointer text-xs font-semibold">Options</summary>
            <div className="grid grid-cols-2 gap-3 mt-2">
              <div>
                <label className={label} htmlFor="opt-overwrite">If the target exists</label>
                <select id="opt-overwrite" className={field} value={form.options.overwrite} onChange={(e) => update({ options: { ...form.options, overwrite: e.target.value as OverwritePolicy } })}>
                  {(Object.keys(OVERWRITE_LABELS) as OverwritePolicy[]).filter((k) => k !== "ask_each_time").map((k) => (
                    <option key={k} value={k}>{OVERWRITE_LABELS[k]}</option>
                  ))}
                </select>
              </div>
              <div>
                <label className={label} htmlFor="opt-retries">Retries on network/busy errors</label>
                <input
                  id="opt-retries"
                  type="number"
                  min={0}
                  max={10}
                  className={field}
                  value={form.options.retry.retries}
                  onChange={(e) => {
                    const retries = Number(e.target.value);
                    update({ options: { ...form.options, retry: retries === 0 ? { ...NO_RETRY } : { ...DEFAULT_RETRY, ...form.options.retry, retries, delay_ms: form.options.retry.delay_ms || DEFAULT_RETRY.delay_ms } } });
                  }}
                />
              </div>
              {fileTrigger && (
                <div>
                  <label className={label} htmlFor="opt-settle">Wait until the file stops changing (s)</label>
                  <input id="opt-settle" type="number" min={0} className={field} value={form.options.settle_secs} onChange={(e) => update({ options: { ...form.options, settle_secs: Number(e.target.value) } })} />
                </div>
              )}
              {advanced && (
                <div>
                  <label className={label} htmlFor="opt-priority">Order (lower runs first)</label>
                  <input id="opt-priority" type="number" className={field} value={form.options.priority} onChange={(e) => update({ options: { ...form.options, priority: Number(e.target.value) } })} />
                </div>
              )}
              {advanced && (
                <label className="flex items-center gap-1 text-xs col-span-2">
                  <input type="checkbox" checked={form.options.allow_overlap} onChange={(e) => update({ options: { ...form.options, allow_overlap: e.target.checked } })} />
                  Allow a new run to start before the previous one finishes
                </label>
              )}
            </div>
          </details>

          {/* Dry run */}
          {preview && (
            <section aria-label="Dry run result" className="rounded border border-[var(--color-border)] p-2 text-xs space-y-1">
              <p className="font-semibold">Dry run: {preview.status === "skipped" ? "no files match" : `${preview.files_affected.length} file(s) would be affected`}</p>
              <ul className="max-h-32 overflow-y-auto font-mono text-[11px]">
                {preview.files_affected.slice(0, 50).map((f) => (
                  <li key={f}>
                    <button type="button" className="hover:underline text-left break-all" onClick={() => setSample(f)} title="Use as preview sample">{f}</button>
                  </li>
                ))}
              </ul>
              {preview.files_affected.length > 50 && <p>…and {preview.files_affected.length - 50} more</p>}
              {preview.error_message && preview.status !== "success" && <p className="text-red-500">{preview.error_message}</p>}
            </section>
          )}
          <div>
            <label className={label} htmlFor="sample-file">Preview templates with this file</label>
            <input id="sample-file" className={field} value={sample} placeholder="/path/to/example.pdf" onChange={(e) => setSample(e.target.value)} />
          </div>

          {/* Approval and webhook (after saving) */}
          {needsApproval && saved && (
            <section className="rounded border border-amber-500/50 bg-amber-500/5 p-2 text-xs space-y-2" aria-label="Command approval">
              <p className="font-semibold">This rule runs programs. Approve exactly these command lines:</p>
              <ul className="font-mono text-[11px]">
                {commandLines(saved).map((c) => (
                  <li key={c}>{c}</li>
                ))}
              </ul>
              <Button
                size="sm"
                onClick={async () => {
                  try {
                    setSaved(await approveCommand(saved.id));
                  } catch (e) {
                    setErrors([errorText(e)]);
                  }
                }}
              >
                <ShieldCheck className="h-3.5 w-3.5 mr-1" /> Approve commands
              </Button>
            </section>
          )}
          {saved && saved.trigger.type === "webhook" && (
            <section className="rounded border border-[var(--color-border)] p-2 text-xs space-y-2" aria-label="Webhook URL">
              {webhook ? (
                <>
                  <p className="font-semibold">Webhook URL (shown once; keep it secret):</p>
                  <input readOnly className={`${field} font-mono`} value={webhook.url} onFocus={(e) => e.currentTarget.select()} aria-label="Webhook URL value" />
                  <p className="text-[11px] text-[var(--color-text-muted)]">POST JSON to it. Listening on this computer only, unless changed in settings.</p>
                </>
              ) : (
                <Button
                  size="sm"
                  variant="outline"
                  onClick={async () => {
                    try {
                      setWebhook(await rotateWebhook(saved.id));
                    } catch (e) {
                      setErrors([errorText(e)]);
                    }
                  }}
                >
                  <Link2 className="h-3.5 w-3.5 mr-1" /> {saved.options?.webhook_secret_hash ? "Create a new URL (the old one stops working)" : "Create webhook URL"}
                </Button>
              )}
            </section>
          )}

          </fieldset>

          {errors.length > 0 && (
            <ul className="text-xs text-red-500 space-y-0.5" role="alert">
              {errors.map((e) => (
                <li key={e}>{e}</li>
              ))}
            </ul>
          )}
        </div>

        <div className="flex items-center justify-between border-t border-[var(--color-border)] px-4 py-3">
          <div className="flex gap-2">
            <Button variant="outline" size="sm" onClick={dryRun} disabled={busy}>
              <FlaskConical className="h-3.5 w-3.5 mr-1" /> Dry run
            </Button>
            <Button variant="ghost" size="sm" onClick={copyJson} title="Copy the rule's definition, e.g. to add it as an organization workflow in the admin console">
              <ClipboardCopy className="h-3.5 w-3.5 mr-1" /> {copied ? "Copied" : "Copy as JSON"}
            </Button>
          </div>
          <div className="flex gap-2">
            <Button variant="ghost" size="sm" onClick={closeEditor}>Close</Button>
            {!managedBy && <Button size="sm" onClick={save} disabled={busy}>{form.id ? "Save changes" : "Create rule"}</Button>}
          </div>
        </div>
      </div>
    </div>
  );
}
