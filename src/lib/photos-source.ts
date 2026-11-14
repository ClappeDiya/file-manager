/** `photos://` job sources (macOS Photos libraries); mirrors Rust `backup::photos`. */

export const PHOTOS_SCHEME = "photos://";

export function isPhotosSource(source: string): boolean {
  return source.trim().startsWith(PHOTOS_SCHEME);
}

/** Same form as Rust `photos::source_uri`. */
export function photosSourceUri(path: string): string {
  const p = path.trim().replace(/[/\\]+$/, "");
  return p.startsWith("/") ? `${PHOTOS_SCHEME}${p}` : `${PHOTOS_SCHEME}/${p}`;
}

/** Library path of a `photos://` source (inverse of {@link photosSourceUri}). */
export function photosLibraryPath(source: string): string {
  const rest = source.trim().slice(PHOTOS_SCHEME.length);
  return /^\/[A-Za-z]:/.test(rest) ? rest.slice(1) : rest;
}
