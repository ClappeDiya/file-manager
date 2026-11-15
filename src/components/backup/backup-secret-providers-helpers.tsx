import { type SecretProvider, type SecretProviderKind, type TlsOptions } from "@/stores/backup-store";


export const TLS: TlsOptions = { accept_invalid_certs: false, ca_pem: null, pinned_sha256: [] };


export const PROVIDER_KINDS: { type: SecretProviderKind["type"]; label: string; credential: string | null }[] = [
  { type: "env", label: "Environment variables", credential: null },
  { type: "file", label: "JSON / .env file", credential: null },
  { type: "pass", label: "pass (Linux password store)", credential: null },
  { type: "vault", label: "HashiCorp Vault (KV)", credential: "Vault token" },
  { type: "aws_secrets_manager", label: "AWS Secrets Manager", credential: "Secret access key (optionally + newline + session token)" },
  { type: "gcp_secret_manager", label: "Google Secret Manager", credential: "Service-account key JSON or access token" },
  { type: "azure_key_vault", label: "Azure Key Vault", credential: "Client secret" },
];


export function newProvider(type: SecretProviderKind["type"]): SecretProvider {
  const base = { id: crypto.randomUUID(), name: PROVIDER_KINDS.find((k) => k.type === type)!.label, credential_id: crypto.randomUUID(), enabled: true, global: true };
  switch (type) {
    case "env":
      return { ...base, type, prefix: "" };
    case "file":
      return { ...base, type, path: "" };
    case "pass":
      return { ...base, type, executable: "", prefix: "" };
    case "vault":
      return { ...base, type, address: "", mount: "secret", path: "", namespace: "", kv_version: 2, tls: TLS };
    case "aws_secrets_manager":
      return { ...base, type, region: "us-east-1", access_key_id: "", endpoint: "", secret_id: "", tls: TLS };
    case "gcp_secret_manager":
      return { ...base, type, project: "", endpoint: "", tls: TLS };
    case "azure_key_vault":
      return { ...base, type, vault_url: "", tenant_id: "", client_id: "", authority: "", tls: TLS };
  }
}
