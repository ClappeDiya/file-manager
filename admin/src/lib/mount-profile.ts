/**
 * The mount profile an organization pushes to its desktops (part of the
 * managed app settings). Mirrors `ufop_core::agent::ManagedMounts`; shared by
 * the settings repository and the App settings page.
 */
export const MOUNT_CACHE_MODES = ['off', 'minimal', 'writes', 'full'] as const;
export const MOUNT_PROTOCOLS = ['rclone', 's3', 'smb', 'nfs', 'sftp', 'webdav', 'ftp'] as const;

export interface ManagedMounts {
  /** Protocols that may be mounted; null = any. */
  allowedProtocols: string[] | null;
  readOnly: boolean | null;
  cacheMode: (typeof MOUNT_CACHE_MODES)[number] | null;
  maxCacheMb: number | null;
  maxCacheAgeHours: number | null;
  /** Mount points must be inside this folder (`~` = home). */
  mountRoot: string | null;
}
