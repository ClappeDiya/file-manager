/** Provider presets served by `connection_presets` (Rust catalog). */

/** Mirrors `connectors::presets::ProviderPreset` (the shared catalog). */
export interface ProviderPreset {
  id: string;
  label: string;
  protocol: string;
  host: string;
  port: number | null;
  default_region: string | null;
  path_style: boolean;
  fields: string[];
  username: string | null;
  help: string;
}

export const PRESET_FIELD_LABELS: Record<string, string> = { region: "Region", account: "Account ID", domain: "Domain", subdomain: "Subdomain" };

/** Fill `{name}` placeholders (unknown ones stay). */
export function fillPreset(template: string, values: Record<string, string>): string {
  return template.replace(/\{(\w+)\}/g, (m, k: string) => values[k] || m);
}
