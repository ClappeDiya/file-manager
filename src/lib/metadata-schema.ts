/** Metadata schema types (Rust: metadata_engine::schema, serde snake_case) and helpers. */

export type FieldKind =
  | "text"
  | "number"
  | "integer"
  | "date"
  | "boolean"
  | "choice"
  | "email"
  | "url";

export interface MetadataField {
  key: string;
  label: string;
  kind: FieldKind;
  required: boolean;
  description: string | null;
  options: string[];
  pattern: string | null;
  min: number | null;
  max: number | null;
}

export interface MetadataSchema {
  id: string;
  name: string;
  folder: string;
  fields: MetadataField[];
  strict: boolean;
  recursive: boolean;
  created_at: string;
  updated_at: string;
}

export interface ComplianceIssue {
  path: string;
  key: string;
  problem: "missing" | "invalid" | "not_allowed";
  message: string;
}

export interface ComplianceReport {
  schema: MetadataSchema | null;
  files_checked: number;
  compliant: number;
  issues: ComplianceIssue[];
  truncated: boolean;
}

export const FIELD_KINDS: [FieldKind, string][] = [
  ["text", "Text"],
  ["number", "Number"],
  ["integer", "Whole number"],
  ["date", "Date"],
  ["boolean", "Yes / no"],
  ["choice", "Choice"],
  ["email", "Email"],
  ["url", "URL"],
];

export const fieldLabel = (f: MetadataField) => f.label || f.key;

export const emptyField = (): MetadataField => ({
  key: "",
  label: "",
  kind: "text",
  required: false,
  description: null,
  options: [],
  pattern: null,
  min: null,
  max: null,
});
