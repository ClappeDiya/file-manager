/**
 * Sign-in for Google Drive, Dropbox and OneDrive connections (mirrors
 * `commands/oauth_commands.rs` and `connectors/oauth_flow.rs`).
 */
import { tauriInvoke } from "@/hooks/use-tauri";

export const OAUTH_PROTOCOLS = new Set(["google_drive", "dropbox", "onedrive"]);

export type OAuthProvider = "google_drive" | "dropbox" | "one_drive";

/** `OAuthProviderInfo` */
export interface OAuthProviderInfo {
  provider: OAuthProvider;
  label: string;
  has_builtin_client: boolean;
  uses_client_secret: boolean;
  redirect_uri: string;
  app_console: string;
}

/** `OAuthStart` */
export interface OAuthStart {
  state: string;
  provider: OAuthProvider;
  auth_url: string;
  redirect_uri: string;
  browser_opened: boolean;
}

/** `OAuthSignedIn` */
export interface OAuthSignedIn {
  connection_id: string;
  provider: OAuthProvider;
  account: string | null;
}

export function providerInfo(protocol: string): Promise<OAuthProviderInfo> {
  return tauriInvoke<OAuthProviderInfo>("oauth_provider_info", { protocol });
}

export function beginSignIn(connectionId: string, clientId?: string | null, clientSecret?: string | null): Promise<OAuthStart> {
  return tauriInvoke<OAuthStart>("oauth_begin", {
    connectionId,
    clientId: clientId?.trim() || null,
    clientSecret: clientSecret?.trim() || null,
    openBrowser: true,
  });
}

export function awaitSignIn(state: string): Promise<OAuthSignedIn> {
  return tauriInvoke<OAuthSignedIn>("oauth_await", { state });
}

export function completeSignIn(state: string, code: string): Promise<OAuthSignedIn> {
  return tauriInvoke<OAuthSignedIn>("oauth_complete", { state, code });
}

export function cancelSignIn(state: string): Promise<void> {
  return tauriInvoke<void>("oauth_cancel", { state }, undefined);
}

/** Headless: pasted tokens, or an AuthID from a self-hosted sign-in broker. */
export function importGrant(connectionId: string, input: { token?: string; brokerUrl?: string; authid?: string }): Promise<OAuthSignedIn> {
  return tauriInvoke<OAuthSignedIn>("oauth_import_grant", {
    connectionId,
    token: input.token?.trim() || null,
    brokerUrl: input.brokerUrl?.trim() || null,
    authid: input.authid?.trim() || null,
  });
}

export function signOut(connectionId: string): Promise<void> {
  return tauriInvoke<void>("oauth_sign_out", { connectionId });
}

export function isSignedIn(connectionId: string): Promise<boolean> {
  return tauriInvoke<boolean>("oauth_status", { connectionId }, false);
}

/** `SharedDriveInfo` */
export interface SharedDrive {
  id: string;
  name: string;
}

/** `SharePointSite` */
export interface SharePointSite {
  id: string;
  display_name: string;
  web_url: string;
}

/** `SharePointLibrary` */
export interface SharePointLibrary {
  id: string;
  name: string;
}

/** `DropboxNamespace` */
export interface DropboxNamespace {
  namespace_id: string | null;
  label: string;
  kind: "home" | "team_space" | "team_folder" | string;
}

export function listSharedDrives(connectionId: string): Promise<SharedDrive[]> {
  return tauriInvoke<SharedDrive[]>("gdrive_list_shared_drives", { connectionId });
}

export function searchSharePointSites(connectionId: string, query: string): Promise<SharePointSite[]> {
  return tauriInvoke<SharePointSite[]>("onedrive_search_sites", { connectionId, query });
}

export function listSiteLibraries(connectionId: string, siteId: string): Promise<SharePointLibrary[]> {
  return tauriInvoke<SharePointLibrary[]>("onedrive_list_site_libraries", { connectionId, siteId });
}

export function listDropboxNamespaces(connectionId: string): Promise<DropboxNamespace[]> {
  return tauriInvoke<DropboxNamespace[]>("dropbox_list_namespaces", { connectionId });
}

/** Formats Google exports each native type to (default first), mirroring `GoogleDocExportFormat::choices`. */
export const GOOGLE_EXPORT_CHOICES = {
  document: ["docx", "pdf", "txt", "html", "epub"],
  spreadsheet: ["xlsx", "pdf", "csv", "html"],
  presentation: ["pptx", "pdf", "txt"],
  drawing: ["pdf"],
} as const;

/** Tell open connection lists to reload (a wizard saved one). */
export const CONNECTIONS_CHANGED_EVENT = "ufop:connections-changed";

export function announceConnectionsChanged(): void {
  window.dispatchEvent(new CustomEvent(CONNECTIONS_CHANGED_EVENT));
}
