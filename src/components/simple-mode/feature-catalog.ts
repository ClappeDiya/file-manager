import {
  GUIDED_FLOW_IDS,
  GUIDED_FLOW_META,
} from "@/components/guided-flows/flow-meta";
/** Discoverable entry points. Destructive file actions stay in the file browser. */
export type FeatureGroup =
  | "Files & folders"
  | "Transfers & protection"
  | "Connections & devices"
  | "Automation & AI"
  | "Security & sharing"
  | "Tools & diagnostics"
  | "Preferences & workspace"
  | "Guided tasks";
export type FeatureTarget =
  | {
      section:
        | "files"
        | "transfers"
        | "backups"
        | "sync"
        | "cloud"
        | "search"
        | "favorites"
        | "activity"
        | "settings";
    }
  | { command: string }
  | { settings: string }
  | { tool: "ai" | "terminal" | "lineage" | "file-actions" };
export interface PlatformFeature {
  id: string;
  label: string;
  description: string;
  group: FeatureGroup;
  target: FeatureTarget;
  selection?: boolean;
}
export const PLATFORM_FEATURES: PlatformFeature[] = [
  {
    id: "files",
    label: "Files & folders",
    description:
      "Source and destination panes, copy, move, rename, delete, archives, permissions and file sharing from the file menu.",
    group: "Files & folders",
    target: { section: "files" },
  },
  {
    id: "search",
    label: "Search",
    description: "Find files and search their contents.",
    group: "Files & folders",
    target: { section: "search" },
  },
  {
    id: "favorites",
    label: "Favorites & recent folders",
    description: "Return to saved locations.",
    group: "Files & folders",
    target: { section: "favorites" },
  },
  {
    id: "preview",
    label: "File preview",
    description:
      "Images, PDF, code, Markdown, audio, video, archives and metadata.",
    group: "Files & folders",
    target: { command: "open-preview" },
    selection: true,
  },
  {
    id: "rename",
    label: "Batch rename",
    description:
      "Preview changes to multiple names, including regular expressions.",
    group: "Files & folders",
    target: { command: "batch-rename" },
    selection: true,
  },
  {
    id: "compare",
    label: "Compare folders",
    description: "Compare the source and destination panes.",
    group: "Files & folders",
    target: { command: "compare-panes" },
  },
  {
    id: "history",
    label: "File history & lineage",
    description:
      "Inspect the selected file’s recorded history and relationships.",
    group: "Files & folders",
    target: { tool: "lineage" },
    selection: true,
  },
  {
    id: "transfers",
    label: "Transfers",
    description:
      "Queue, pause, cancel and retry transfers; review progress and results.",
    group: "Transfers & protection",
    target: { section: "transfers" },
  },
  {
    id: "transfer-history",
    label: "Transfer history",
    description: "Review previous transfers and verification results.",
    group: "Transfers & protection",
    target: { command: "open-transfer-history" },
  },
  {
    id: "backups",
    label: "Backups & restore",
    description:
      "Versioned, encrypted backups, restore, verification, repair and schedules.",
    group: "Transfers & protection",
    target: { section: "backups" },
  },
  {
    id: "sync",
    label: "Folder sync",
    description: "Sync pairs, dry runs, conflicts, reports and rollback.",
    group: "Transfers & protection",
    target: { section: "sync" },
  },
  {
    id: "migration",
    label: "Migration wizard",
    description: "Plan and run a migration between locations.",
    group: "Transfers & protection",
    target: { command: "open-migration" },
  },
  {
    id: "integrity",
    label: "Integrity & duplicates",
    description: "Checksums, duplicate detection and smart folders.",
    group: "Transfers & protection",
    target: { command: "open-integrity-tools" },
  },
  {
    id: "connections",
    label: "Cloud & servers",
    description: "Create, test, import and export connection profiles.",
    group: "Connections & devices",
    target: { section: "cloud" },
  },
  {
    id: "rclone",
    label: "rclone remotes",
    description: "Connect additional cloud providers and encrypted remotes.",
    group: "Connections & devices",
    target: { command: "open-rclone-remotes" },
  },
  {
    id: "rclone-setup",
    label: "rclone setup",
    description:
      "Install a verified managed copy or choose an existing program and config.",
    group: "Connections & devices",
    target: { settings: "rclone" },
  },
  {
    id: "nearby",
    label: "Nearby computers",
    description: "Discover devices, send and receive files.",
    group: "Connections & devices",
    target: { command: "open-nearby" },
  },
  {
    id: "mounts",
    label: "Mounted drives",
    description: "Mount connections as drives and manage current mounts.",
    group: "Connections & devices",
    target: { command: "open-mounts" },
  },
  {
    id: "managed",
    label: "Managed console & shared workspaces",
    description:
      "Enroll this device and receive assigned connectors and workspace folders.",
    group: "Connections & devices",
    target: { settings: "managed" },
  },
  {
    id: "quickflows",
    label: "Quickflows & automation",
    description: "Create, manage and run automation rules.",
    group: "Automation & AI",
    target: { command: "toggle-automations" },
  },
  {
    id: "smart-spaces",
    label: "Smart Spaces",
    description: "Create rule-based collections of files.",
    group: "Automation & AI",
    target: { command: "create-smart-space" },
  },
  {
    id: "schedules",
    label: "Schedules & calendars",
    description: "Reusable schedules and holiday calendars.",
    group: "Automation & AI",
    target: { command: "open-schedules" },
  },
  {
    id: "ai",
    label: "AI assistant",
    description: "Review and run AI-assisted file operations.",
    group: "Automation & AI",
    target: { tool: "ai" },
  },
  {
    id: "ai-settings",
    label: "AI providers & intent bar",
    description: "Configure providers, API credentials and intent suggestions.",
    group: "Automation & AI",
    target: { settings: "ai" },
  },
  {
    id: "vault",
    label: "Encrypted vault",
    description: "Manage encrypted file storage.",
    group: "Security & sharing",
    target: { command: "open-vault" },
  },
  {
    id: "lock",
    label: "App lock",
    description: "Configure this device’s lock and unlock protection.",
    group: "Security & sharing",
    target: { settings: "app-lock" },
  },
  {
    id: "sharing",
    label: "Sharing settings",
    description: "Configure the sharing server and public link policy.",
    group: "Security & sharing",
    target: { settings: "sharing" },
  },
  {
    id: "gpg",
    label: "OpenPGP encryption & signatures",
    description:
      "Manage keys; encrypt, decrypt, sign and verify selected files from their file menu.",
    group: "Security & sharing",
    target: { settings: "gpg" },
  },
  {
    id: "audit",
    label: "Security audit log",
    description: "Inspect recorded security events.",
    group: "Security & sharing",
    target: { settings: "audit" },
  },
  {
    id: "ssh",
    label: "Trusted SSH hosts & certificates",
    description: "Review trusted hosts and host certificate authorities.",
    group: "Security & sharing",
    target: { settings: "ssh" },
  },
  {
    id: "kerberos",
    label: "Kerberos / GSSAPI",
    description: "Configure integrated network authentication.",
    group: "Security & sharing",
    target: { settings: "kerberos" },
  },
  {
    id: "privacy",
    label: "Privacy",
    description: "Control privacy preferences.",
    group: "Security & sharing",
    target: { settings: "privacy" },
  },
  {
    id: "terminal",
    label: "Terminal",
    description: "Open a terminal session for commands and scripts.",
    group: "Tools & diagnostics",
    target: { tool: "terminal" },
  },
  {
    id: "file-tools",
    label: "Archives, conversion & file actions",
    description:
      "Select files in Files, then open their context menu for supported conversion, archive, OpenPGP, multi-destination copy and metadata actions.",
    group: "Tools & diagnostics",
    target: { tool: "file-actions" },
  },
  {
    id: "logs",
    label: "Log viewer",
    description: "Inspect application logs and diagnostics.",
    group: "Tools & diagnostics",
    target: { command: "open-log-viewer" },
  },
  {
    id: "activity",
    label: "Activity",
    description: "Review recent operations.",
    group: "Tools & diagnostics",
    target: { section: "activity" },
  },
  {
    id: "feedback",
    label: "Help & feedback",
    description: "Report a problem with diagnostic details you choose.",
    group: "Tools & diagnostics",
    target: { command: "send-feedback" },
  },
  {
    id: "index",
    label: "Search index",
    description: "Manage indexing and content search.",
    group: "Tools & diagnostics",
    target: { settings: "index" },
  },
  {
    id: "settings",
    label: "Sidebar layout & appearance",
    description: "Apply Basic or Advanced navigation and choose a theme.",
    group: "Preferences & workspace",
    target: { section: "settings" },
  },
  {
    id: "general",
    label: "General preferences",
    description: "Language, defaults and application behavior.",
    group: "Preferences & workspace",
    target: { settings: "general" },
  },
  {
    id: "startup",
    label: "Startup & tray",
    description: "Choose startup and background behavior.",
    group: "Preferences & workspace",
    target: { settings: "startup" },
  },
  {
    id: "workspace",
    label: "Save workspace",
    description:
      "Save the current panes and tabs. Load a saved workspace from the Files toolbar.",
    group: "Preferences & workspace",
    target: { command: "save-workspace" },
  },
  {
    id: "settings-sync",
    label: "Settings backup & device sync",
    description: "Export, import and synchronize preferences between devices.",
    group: "Preferences & workspace",
    target: { settings: "settings-sync" },
  },
  {
    id: "storage",
    label: "Storage & portable mode",
    description: "Review local data locations and portable storage.",
    group: "Preferences & workspace",
    target: { settings: "storage" },
  },
  {
    id: "names",
    label: "Filename policies",
    description: "Configure compatibility and naming rules.",
    group: "Preferences & workspace",
    target: { settings: "names" },
  },
  {
    id: "proxy",
    label: "Network & proxy",
    description: "Configure proxy settings for connections.",
    group: "Preferences & workspace",
    target: { settings: "proxy" },
  },
  {
    id: "editors",
    label: "External editors",
    description: "Choose programs for opening and editing files.",
    group: "Preferences & workspace",
    target: { settings: "editors" },
  },
  {
    id: "notifications",
    label: "Notifications",
    description: "Choose notification behavior.",
    group: "Preferences & workspace",
    target: { settings: "notifications" },
  },
  {
    id: "ssh-cas",
    label: "Trusted SSH certificate authorities",
    description: "Manage trusted host certificate authorities.",
    group: "Security & sharing",
    target: { settings: "ssh-cas" },
  },
  {
    id: "export-settings",
    label: "Export settings",
    description: "Save preferences for backup or another installation.",
    group: "Preferences & workspace",
    target: { settings: "export" },
  },
  {
    id: "import-settings",
    label: "Import settings",
    description: "Review and import a preferences file.",
    group: "Preferences & workspace",
    target: { settings: "import" },
  },
  {
    id: "toolbar",
    label: "Customize file toolbar",
    description: "Choose which tools appear above the file panes.",
    group: "Preferences & workspace",
    target: { command: "customize-toolbar" },
  },
  {
    id: "network",
    label: "Network setup",
    description: "Network settings and connection setup.",
    group: "Connections & devices",
    target: { settings: "network" },
  },
  {
    id: "logging",
    label: "Logging & diagnostics",
    description: "Configure logging and collect diagnostics.",
    group: "Tools & diagnostics",
    target: { settings: "logging" },
  },
  {
    id: "advanced",
    label: "Advanced settings",
    description: "Review additional application configuration.",
    group: "Preferences & workspace",
    target: { settings: "advanced" },
  },
  ...GUIDED_FLOW_IDS.map((id) => ({
    id: `guided-${id}`,
    label: GUIDED_FLOW_META[id].label,
    description: GUIDED_FLOW_META[id].description,
    group: "Guided tasks" as const,
    target: { command: `guided-${id}` },
  })),
];
