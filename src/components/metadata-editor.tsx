/**
 * Key-value file metadata (business identifiers, classifications, notes)
 * and search by metadata. Values persist in SQLite on the Rust side
 * (metadata_engine) and follow files that are moved or renamed in the app.
 * When a folder schema governs the file, its fields come first with typed
 * inputs; a strict schema allows no other keys.
 */
import { useCallback, useEffect, useState } from "react";
import { tauriInvoke, tauriInvokeSafe } from "@/hooks/use-tauri";
import { SchemaFieldInput } from "@/components/metadata-schema";
import { fieldLabel, type MetadataSchema } from "@/lib/metadata-schema";

export interface FileLabel {
  path: string;
  tags: string[];
  labels: string[];
  metadata: Record<string, string>;
}

export interface MetadataMatch {
  path: string;
  key: string;
  value: string;
}

const inputClass =
  "px-2 py-1 text-sm border rounded bg-surface-primary border-border-primary min-w-0";

export function MetadataEditor({
  path,
  onChange,
}: {
  path: string;
  onChange?: (info: FileLabel) => void;
}) {
  const [metadata, setMetadata] = useState<Record<string, string>>({});
  const [keys, setKeys] = useState<string[]>([]);
  const [schema, setSchema] = useState<MetadataSchema | null>(null);
  const [newKey, setNewKey] = useState("");
  const [newValue, setNewValue] = useState("");
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    const info = await tauriInvokeSafe<FileLabel | null>(
      "integrity_get_file_info",
      { path },
      null,
    );
    setMetadata(info?.metadata ?? {});
    setSchema(
      await tauriInvokeSafe<MetadataSchema | null>(
        "metadata_schema_for",
        { path },
        null,
      ),
    );
    setKeys(
      await tauriInvokeSafe<string[]>("integrity_metadata_keys", undefined, []),
    );
  }, [path]);

  useEffect(() => {
    void load();
  }, [load]);

  const apply = async (command: string, args: Record<string, string>) => {
    setError("");
    try {
      const info = await tauriInvoke<FileLabel>(command, { path, ...args });
      setMetadata(info.metadata ?? {});
      onChange?.(info);
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  };

  const add = async () => {
    if (!newKey.trim()) return;
    if (
      await apply("integrity_set_metadata", {
        key: newKey.trim(),
        value: newValue,
      })
    ) {
      setNewKey("");
      setNewValue("");
    }
  };

  const schemaKeys = new Set(schema?.fields.map((f) => f.key));
  const entries = Object.entries(metadata).filter(([k]) => !schemaKeys.has(k));
  const setValue = (key: string, value: string) =>
    value === ""
      ? metadata[key] !== undefined
        ? apply("integrity_remove_metadata", { key })
        : undefined
      : apply("integrity_set_metadata", { key, value });
  return (
    <div className="space-y-2" data-testid="metadata-editor">
      <h4 className="text-xs font-medium text-text-secondary">Metadata</h4>
      {schema && (
        <div className="space-y-1" aria-label={`Schema ${schema.name}`}>
          <p className="text-xs text-text-tertiary">
            Schema: <span className="font-medium">{schema.name}</span>
            {schema.strict && " (only these fields)"}
          </p>
          <dl className="space-y-1">
            {schema.fields.map((f) => {
              const missing = f.required && !(metadata[f.key] ?? "").trim();
              return (
                <div key={f.key} className="flex items-center gap-2 text-xs">
                  <dt
                    className={`shrink-0 w-28 truncate ${missing ? "text-amber-600" : "text-text-secondary"}`}
                    title={f.description ?? f.key}
                  >
                    {fieldLabel(f)}
                    {f.required && <span aria-label="required"> *</span>}
                  </dt>
                  <dd className="flex-1 min-w-0">
                    <SchemaFieldInput
                      field={f}
                      value={metadata[f.key] ?? ""}
                      onCommit={(v) => setValue(f.key, v)}
                      className="w-full"
                    />
                  </dd>
                </div>
              );
            })}
          </dl>
        </div>
      )}
      {!schema && entries.length === 0 && (
        <p className="text-xs text-text-tertiary">No metadata on this file.</p>
      )}
      <dl className="space-y-1">
        {entries.map(([key, value]) => (
          <div key={key} className="flex items-center gap-2 text-xs">
            <dt className="font-mono text-text-secondary shrink-0">{key}</dt>
            <dd className="flex-1 min-w-0">
              <input
                aria-label={`Value for ${key}`}
                defaultValue={value}
                onBlur={(e) =>
                  e.target.value !== value &&
                  apply("integrity_set_metadata", {
                    key,
                    value: e.target.value,
                  })
                }
                className={`${inputClass} w-full`}
              />
            </dd>
            <button
              type="button"
              aria-label={`Remove ${key}`}
              onClick={() => apply("integrity_remove_metadata", { key })}
              className="text-text-tertiary hover:text-red-500"
            >
              x
            </button>
          </div>
        ))}
      </dl>
      {!schema?.strict && (
        <div className="flex gap-2">
          <input
            list="metadata-keys"
            aria-label="New metadata key"
            placeholder="Key (e.g. invoice)"
            value={newKey}
            onChange={(e) => setNewKey(e.target.value)}
            className={`${inputClass} w-32`}
          />
          <datalist id="metadata-keys">
            {keys.map((k) => (
              <option key={k} value={k} />
            ))}
          </datalist>
          <input
            aria-label="New metadata value"
            placeholder="Value"
            value={newValue}
            onChange={(e) => setNewValue(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && add()}
            className={`${inputClass} flex-1`}
          />
          <button
            type="button"
            onClick={add}
            disabled={!newKey.trim()}
            className="px-3 py-1 text-sm rounded bg-accent-primary text-white disabled:opacity-50"
          >
            Set
          </button>
        </div>
      )}
      {error && (
        <p className="text-xs text-red-600 dark:text-red-400">{error}</p>
      )}
    </div>
  );
}

export function MetadataSearch({
  under,
  onOpen,
}: {
  under?: string;
  onOpen?: (path: string) => void;
}) {
  const [keys, setKeys] = useState<string[]>([]);
  const [key, setKey] = useState("");
  const [value, setValue] = useState("");
  const [onlyHere, setOnlyHere] = useState(Boolean(under));
  const [results, setResults] = useState<MetadataMatch[] | null>(null);
  const [error, setError] = useState("");

  useEffect(() => {
    void tauriInvokeSafe<string[]>(
      "integrity_metadata_keys",
      undefined,
      [],
    ).then(setKeys);
  }, []);

  const search = async () => {
    setError("");
    try {
      setResults(
        await tauriInvoke<MetadataMatch[]>("integrity_search_metadata", {
          key: key || null,
          value,
          contains: true,
          under: onlyHere && under ? under : null,
          limit: 500,
        }),
      );
    } catch (e) {
      setError(String(e));
    }
  };

  return (
    <div className="space-y-2" data-testid="metadata-search">
      <h3 className="text-sm font-medium">Find by metadata</h3>
      <div className="flex flex-wrap gap-2">
        <select
          aria-label="Metadata key"
          value={key}
          onChange={(e) => setKey(e.target.value)}
          className={inputClass}
        >
          <option value="">Any key</option>
          {keys.map((k) => (
            <option key={k} value={k}>
              {k}
            </option>
          ))}
        </select>
        <input
          aria-label="Metadata value contains"
          placeholder="Value contains…"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && search()}
          className={`${inputClass} flex-1`}
        />
        <button
          type="button"
          onClick={search}
          className="px-3 py-1 text-sm rounded bg-accent-primary text-white"
        >
          Search
        </button>
      </div>
      {under && (
        <label className="flex items-center gap-2 text-xs text-text-secondary">
          <input
            type="checkbox"
            checked={onlyHere}
            onChange={(e) => setOnlyHere(e.target.checked)}
          />
          Only in this folder
        </label>
      )}
      {results &&
        (results.length === 0 ? (
          <p className="text-xs text-text-tertiary">No files match.</p>
        ) : (
          <ul
            className="text-xs space-y-1 max-h-64 overflow-y-auto"
            aria-label="Metadata matches"
          >
            {results.map((r) => (
              <li key={`${r.path}:${r.key}`} className="flex gap-2">
                <button
                  type="button"
                  onClick={() => onOpen?.(r.path)}
                  className="font-mono text-accent-primary hover:underline truncate min-w-0 text-left"
                  title={r.path}
                >
                  {r.path}
                </button>
                <span className="text-text-tertiary shrink-0">
                  {r.key} = {r.value}
                </span>
              </li>
            ))}
          </ul>
        ))}
      {error && (
        <p className="text-xs text-red-600 dark:text-red-400">{error}</p>
      )}
    </div>
  );
}
