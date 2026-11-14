/**
 * Types and helpers for cloud destination sign-in settings (the S3
 * credential chain and assume-role, Azure Entra ID sign-in, storage class
 * per volume kind). Shapes mirror the Rust configs (serde snake_case,
 * `type` tags); see `backup-cloud-advanced.tsx` for the editors.
 */
import type { CloudConfig } from "@/stores/backup-store";

export type Obj = Record<string, unknown>;

/** `CredentialSource` (connectors/aws_chain.rs). */
export type S3Credentials =
  | { type: "static" }
  | { type: "environment" }
  | { type: "profile"; name: string; file: string | null }
  | { type: "web_identity"; role_arn: string; token_file: string }
  | { type: "instance_metadata" }
  | { type: "auto" };

/** `AssumeRoleOptions`: `AssumeRoleConfig` fields plus endpoint and region. */
export interface S3AssumeRole {
  role_arn: string;
  external_id: string | null;
  session_name: string | null;
  duration_secs: number | null;
  sts_endpoint: string | null;
  region: string | null;
}

/** `AzureAuth` (stores/azure.rs). */
export type AzureAuth =
  | { type: "shared_key" }
  | { type: "sas" }
  | { type: "service_principal_secret"; tenant: string; client_id: string }
  | { type: "service_principal_certificate"; tenant: string; client_id: string; cert_path: string }
  | { type: "federated_token"; tenant: string; client_id: string; token_file: string }
  | { type: "managed_identity"; client_id: string | null };

/** `ClassByKind` (stores/mod.rs): empty = the destination's single class. */
export interface ClassByKind {
  dblock: string;
  dindex: string;
  dlist: string;
  other: string;
}

export const EMPTY_CLASS_BY_KIND: ClassByKind = { dblock: "", dindex: "", dlist: "", other: "" };

export const S3_CREDENTIAL_LABELS: Record<S3Credentials["type"], string> = {
  static: "Access key (id here, secret below)",
  environment: "Environment variables (AWS_ACCESS_KEY_ID …)",
  profile: "AWS profile (~/.aws/credentials, ~/.aws/config)",
  web_identity: "Web identity token file (EKS, GitHub Actions OIDC)",
  instance_metadata: "EC2 instance profile (IMDSv2)",
  auto: "Automatic (environment → web identity → profile → container → instance)",
};

export const AZURE_AUTH_LABELS: Record<AzureAuth["type"], string> = {
  shared_key: "Account key (or SAS token) below",
  sas: "SAS token below",
  service_principal_secret: "Service principal with a client secret",
  service_principal_certificate: "Service principal with a certificate",
  federated_token: "Workload identity (federated token file)",
  managed_identity: "Managed identity of this machine",
};

/** A fresh value for a credential source type, keeping fields that carry over. */
export function s3CredentialsOf(type: S3Credentials["type"], prev?: S3Credentials): S3Credentials {
  const p = (prev ?? { type: "static" }) as Obj;
  switch (type) {
    case "profile":
      return { type, name: String(p.name ?? ""), file: (p.file as string | null) ?? null };
    case "web_identity":
      return { type, role_arn: String(p.role_arn ?? ""), token_file: String(p.token_file ?? "") };
    default:
      return { type } as S3Credentials;
  }
}

export function azureAuthOf(type: AzureAuth["type"], prev?: AzureAuth): AzureAuth {
  const p = (prev ?? { type: "shared_key" }) as Obj;
  const ids = { tenant: String(p.tenant ?? ""), client_id: String(p.client_id ?? "") };
  switch (type) {
    case "service_principal_secret":
      return { type, ...ids };
    case "service_principal_certificate":
      return { type, ...ids, cert_path: String(p.cert_path ?? "") };
    case "federated_token":
      return { type, ...ids, token_file: String(p.token_file ?? "") };
    case "managed_identity":
      return { type, client_id: (p.client_id as string) || null };
    default:
      return { type } as AzureAuth;
  }
}

/** What the credentials box holds for a sign-in method (empty = nothing to store). */
export function secretHint(config: CloudConfig): string | null {
  if (config.provider === "s3") {
    const t = ((config.credentials as S3Credentials | undefined) ?? { type: "static" }).type;
    return t === "static" ? null : "Not needed: the keys come from the selected credential source.";
  }
  if (config.provider === "azure") {
    const t = ((config.auth as AzureAuth | undefined) ?? { type: "shared_key" }).type;
    if (t === "service_principal_secret") return "The client secret of the app registration.";
    if (t === "service_principal_certificate") return "The PFX password, if the certificate file is a password-protected PFX.";
    if (t === "federated_token" || t === "managed_identity") return "Not needed for this sign-in method.";
  }
  return null;
}

