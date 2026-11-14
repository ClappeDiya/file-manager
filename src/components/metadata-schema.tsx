/**
 * Metadata schemas: typed, optionally required fields that a folder's files
 * carry (Rust: metadata_engine::schema). The panel defines the schema of the
 * current folder and checks its files; `SchemaFieldInput` is the typed input
 * the metadata editor uses for schema fields.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import {
  FIELD_KINDS,
  emptyField,
  type ComplianceReport,
  type FieldKind,
  type MetadataField,
  type MetadataSchema,
} from "@/lib/metadata-schema";

const inputClass =
  "px-2 py-1 text-sm border rounded bg-surface-primary border-border-primary min-w-0";

/** A typed input for one schema field; commits on change (select/checkbox/date) or blur (text). */
export function SchemaFieldInput({
  field,
  value,
  onCommit,
  className = "",
}: {
  field: MetadataField;
  value: string;
  onCommit: (value: string) => void;
  className?: string;
}) {
  const label = `Value for ${field.key}`;
  if (field.kind === "boolean") {
    return (
      <input
        type="checkbox"
        aria-label={label}
        checked={value === "true"}
        onChange={(e) => onCommit(e.target.checked ? "true" : "false")}
      />
    );
  }
  if (field.kind === "choice") {
    return (
      <select
        aria-label={label}
        value={value}
        onChange={(e) => onCommit(e.target.value)}
        className={`${inputClass} ${className}`}
      >
        <option value="">{field.required ? "Choose…" : "—"}</option>
        {field.options.map((o) => (
          <option key={o} value={o}>
            {o}
          </option>
        ))}
      </select>
    );
  }
  const type =
    {
      date: "date",
      number: "number",
      integer: "number",
      email: "email",
      url: "url",
    }[field.kind as string] ?? "text";
  return (
    <input
      key={value}
      type={type}
      aria-label={label}
      defaultValue={value}
      step={field.kind === "integer" ? 1 : undefined}
      min={
        field.kind === "number" || field.kind === "integer"
          ? (field.min ?? undefined)
          : undefined
      }
      max={
        field.kind === "number" || field.kind === "integer"
          ? (field.max ?? undefined)
          : undefined
      }
      placeholder={field.description ?? undefined}
      onBlur={(e) => e.target.value !== value && onCommit(e.target.value)}
      className={`${inputClass} ${className}`}
    />
  );
}

/** Define the schema of `folder` and check its files. */
export function MetadataSchemaPanel({ folder }: { folder: string }) {
  const [schema, setSchema] = useState<MetadataSchema | null>(null);
  const [draft, setDraft] = useState<MetadataSchema | null>(null);
  const [report, setReport] = useState<ComplianceReport | null>(null);
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);

  const load = useCallback(async () => {
    setReport(null);
    const s = await tauriInvokeSafe<MetadataSchema | null>(
      "metadata_schema_for",
      { path: folder, isDir: true },
      null,
    );
    setSchema(s);
    setDraft(null);
  }, [folder]);

  useEffect(() => {
    void load();
  }, [load]);

  const inherited = schema && schema.folder !== folder;
  const startEdit = () =>
    setDraft(
      schema && !inherited
        ? structuredClone(schema)
        : {
            id: "",
            name: folder.split(/[\\/]/).filter(Boolean).pop() ?? "Schema",
            folder,
            fields: [emptyField()],
            strict: false,
            recursive: true,
            created_at: "",
            updated_at: "",
          },
    );

  const run = async (f: () => Promise<void>) => {
    setError("");
    setBusy(true);
    try {
      await f();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      if (!draft) return;
      const saved = await tauriInvoke<MetadataSchema>("metadata_save_schema", {
        schema: draft,
      });
      setSchema(saved);
      setDraft(null);
    });
  const remove = () =>
    run(async () => {
      if (!schema) return;
      await tauriInvoke("metadata_delete_schema", { id: schema.id });
      await load();
    });
  const check = () =>
    run(async () =>
      setReport(
        await tauriInvoke<ComplianceReport>("metadata_check_compliance", {
          folder,
          recursive: true,
        }),
      ),
    );

  const setField = (i: number, patch: Partial<MetadataField>) =>
    draft &&
    setDraft({
      ...draft,
      fields: draft.fields.map((f, j) => (j === i ? { ...f, ...patch } : f)),
    });
  const num = (v: string) => (v.trim() === "" ? null : Number(v));

  return (
    <div className="p-4 space-y-4" data-testid="metadata-schema-panel">
      <div>
        <h3 className="text-sm font-medium">Metadata schema</h3>
        <p className="text-xs text-text-tertiary">
          Fields that files in{" "}
          <span className="font-mono">{folder || "this folder"}</span> carry.
          Values are checked when they are set.
        </p>
      </div>

      {!draft && (
        <div className="space-y-2">
          {schema ? (
            <div className="text-sm space-y-1">
              <div>
                <span className="font-medium">{schema.name}</span>
                {inherited && (
                  <span className="text-xs text-text-tertiary">
                    {" "}
                    — inherited from{" "}
                    <span className="font-mono">{schema.folder}</span>
                  </span>
                )}
                {schema.strict && (
                  <span className="ml-2 text-xs rounded bg-surface-secondary px-1.5">
                    strict
                  </span>
                )}
              </div>
              <ul
                className="text-xs text-text-secondary"
                aria-label="Schema fields"
              >
                {schema.fields.map((f) => (
                  <li key={f.key}>
                    <span className="font-mono">{f.key}</span> ·{" "}
                    {FIELD_KINDS.find(([k]) => k === f.kind)?.[1]}
                    {f.required && " · required"}
                    {f.kind === "choice" && ` (${f.options.join(", ")})`}
                  </li>
                ))}
              </ul>
            </div>
          ) : (
            <p className="text-xs text-text-tertiary">
              No schema applies here; metadata is free-form.
            </p>
          )}
          <div className="flex gap-2">
            <button
              type="button"
              disabled={!folder}
              onClick={startEdit}
              className="px-3 py-1 text-sm rounded bg-accent-primary text-white disabled:opacity-50"
            >
              {schema && !inherited
                ? "Edit schema"
                : inherited
                  ? "Override for this folder"
                  : "Create schema"}
            </button>
            {schema && !inherited && (
              <button
                type="button"
                onClick={remove}
                disabled={busy}
                className="px-3 py-1 text-sm rounded border border-border-primary"
              >
                Delete schema
              </button>
            )}
            {schema && (
              <button
                type="button"
                onClick={check}
                disabled={busy}
                className="px-3 py-1 text-sm rounded border border-border-primary"
              >
                Check files
              </button>
            )}
          </div>
        </div>
      )}

      {draft && (
        <div className="space-y-3" aria-label="Schema editor">
          <input
            aria-label="Schema name"
            value={draft.name}
            onChange={(e) => setDraft({ ...draft, name: e.target.value })}
            className={`${inputClass} w-full`}
          />
          <div className="flex gap-4 text-xs">
            <label className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={draft.recursive}
                onChange={(e) =>
                  setDraft({ ...draft, recursive: e.target.checked })
                }
              />
              Include subfolders
            </label>
            <label className="flex items-center gap-1">
              <input
                type="checkbox"
                checked={draft.strict}
                onChange={(e) =>
                  setDraft({ ...draft, strict: e.target.checked })
                }
              />
              Only these fields (strict)
            </label>
          </div>
          {draft.fields.map((f, i) => (
            <fieldset
              key={i}
              className="border border-border-primary rounded p-2 space-y-1"
            >
              <legend className="text-xs px-1">Field {i + 1}</legend>
              <div className="flex flex-wrap gap-2">
                <input
                  aria-label={`Field ${i + 1} key`}
                  placeholder="key"
                  value={f.key}
                  onChange={(e) => setField(i, { key: e.target.value })}
                  className={`${inputClass} w-28 font-mono`}
                />
                <input
                  aria-label={`Field ${i + 1} label`}
                  placeholder="Label"
                  value={f.label}
                  onChange={(e) => setField(i, { label: e.target.value })}
                  className={`${inputClass} w-32`}
                />
                <select
                  aria-label={`Field ${i + 1} type`}
                  value={f.kind}
                  onChange={(e) =>
                    setField(i, { kind: e.target.value as FieldKind })
                  }
                  className={inputClass}
                >
                  {FIELD_KINDS.map(([k, l]) => (
                    <option key={k} value={k}>
                      {l}
                    </option>
                  ))}
                </select>
                <label className="flex items-center gap-1 text-xs">
                  <input
                    type="checkbox"
                    aria-label={`Field ${i + 1} required`}
                    checked={f.required}
                    onChange={(e) =>
                      setField(i, { required: e.target.checked })
                    }
                  />
                  Required
                </label>
                <button
                  type="button"
                  aria-label={`Remove field ${i + 1}`}
                  onClick={() =>
                    setDraft({
                      ...draft,
                      fields: draft.fields.filter((_, j) => j !== i),
                    })
                  }
                  className="text-text-tertiary hover:text-red-500 text-xs"
                >
                  Remove
                </button>
              </div>
              {f.kind === "choice" && (
                <input
                  aria-label={`Field ${i + 1} options`}
                  placeholder="Options, comma-separated"
                  value={f.options.join(", ")}
                  onChange={(e) =>
                    setField(i, {
                      options: e.target.value
                        .split(",")
                        .map((o) => o.trimStart()),
                    })
                  }
                  className={`${inputClass} w-full`}
                />
              )}
              {f.kind === "text" && (
                <input
                  aria-label={`Field ${i + 1} pattern`}
                  placeholder="Pattern (regular expression, optional)"
                  value={f.pattern ?? ""}
                  onChange={(e) =>
                    setField(i, { pattern: e.target.value || null })
                  }
                  className={`${inputClass} w-full font-mono`}
                />
              )}
              {(f.kind === "number" ||
                f.kind === "integer" ||
                f.kind === "text") && (
                <div className="flex gap-2 text-xs items-center">
                  {f.kind === "text" ? "Length" : "Value"}
                  <input
                    aria-label={`Field ${i + 1} min`}
                    type="number"
                    placeholder="min"
                    value={f.min ?? ""}
                    onChange={(e) => setField(i, { min: num(e.target.value) })}
                    className={`${inputClass} w-20`}
                  />
                  <input
                    aria-label={`Field ${i + 1} max`}
                    type="number"
                    placeholder="max"
                    value={f.max ?? ""}
                    onChange={(e) => setField(i, { max: num(e.target.value) })}
                    className={`${inputClass} w-20`}
                  />
                </div>
              )}
            </fieldset>
          ))}
          <div className="flex gap-2">
            <button
              type="button"
              onClick={() =>
                setDraft({ ...draft, fields: [...draft.fields, emptyField()] })
              }
              className="px-3 py-1 text-sm rounded border border-border-primary"
            >
              Add field
            </button>
            <button
              type="button"
              onClick={save}
              disabled={busy}
              className="px-3 py-1 text-sm rounded bg-accent-primary text-white disabled:opacity-50"
            >
              Save schema
            </button>
            <button
              type="button"
              onClick={() => setDraft(null)}
              className="px-3 py-1 text-sm rounded border border-border-primary"
            >
              Cancel
            </button>
          </div>
        </div>
      )}

      {report && (
        <div className="space-y-1" aria-label="Compliance report">
          <p className="text-sm">
            {report.compliant} of {report.files_checked} files comply
            {report.truncated ? " (first files only)" : ""}.
          </p>
          {report.issues.length > 0 && (
            <ul className="text-xs space-y-0.5 max-h-64 overflow-y-auto">
              {report.issues.map((i) => (
                <li
                  key={`${i.path}:${i.key}:${i.problem}`}
                  className="flex gap-2"
                >
                  <span className="font-mono truncate min-w-0" title={i.path}>
                    {i.path.split(/[\\/]/).pop()}
                  </span>
                  <span
                    className={
                      i.problem === "missing"
                        ? "text-amber-600"
                        : "text-red-600 dark:text-red-400"
                    }
                  >
                    {i.message}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {error && (
        <p className="text-xs text-red-600 dark:text-red-400">{error}</p>
      )}
    </div>
  );
}
