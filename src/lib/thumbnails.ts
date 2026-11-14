/**
 * Thumbnails for the grid view: asks Rust (`thumbnail_get`) for a cached
 * image and keeps the resulting URLs in a small in-memory LRU, sharing
 * in-flight requests so a re-render doesn't ask twice.
 */
import { useEffect, useState } from "react";
import { convertFileSrc } from "@tauri-apps/api/core";
import { isTauriAvailable, tauriInvoke } from "@/hooks/use-tauri";
import { isRemotePath } from "@/lib/location";

/** Mirrors `IMAGE_EXTENSIONS` in thumbnail_engine. */
export const THUMBNAIL_EXTENSIONS = new Set(["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"]);
export const MEMORY_ENTRIES = 500;

export function hasThumbnail(name: string): boolean {
  const ext = name.includes(".") ? name.split(".").pop()!.toLowerCase() : "";
  return THUMBNAIL_EXTENSIONS.has(ext);
}

/** Least-recently-used map: reads move an entry to the back. */
export class LruMap<K, V> {
  private map = new Map<K, V>();
  constructor(private readonly cap: number) {}
  get(k: K): V | undefined {
    if (!this.map.has(k)) return undefined;
    const v = this.map.get(k)!;
    this.map.delete(k);
    this.map.set(k, v);
    return v;
  }
  has(k: K) {
    return this.map.has(k);
  }
  set(k: K, v: V) {
    this.map.delete(k);
    this.map.set(k, v);
    while (this.map.size > this.cap) this.map.delete(this.map.keys().next().value as K);
  }
  get size() {
    return this.map.size;
  }
  clear() {
    this.map.clear();
  }
}

const urls = new LruMap<string, string | null>(MEMORY_ENTRIES);
const inflight = new Map<string, Promise<string | null>>();

/** URL of a thumbnail (null = none). `version` (e.g. mtime) busts the memory cache. */
export function loadThumbnail(path: string, size: number, version = ""): Promise<string | null> {
  const key = `${path}|${size}|${version}`;
  if (urls.has(key)) return Promise.resolve(urls.get(key) ?? null);
  const pending = inflight.get(key);
  if (pending) return pending;
  const p = tauriInvoke<string | null>("thumbnail_get", { path, size })
    .then((file) => (file ? convertFileSrc(file) : null))
    .catch(() => null)
    .then((url) => {
      urls.set(key, url);
      inflight.delete(key);
      return url;
    });
  inflight.set(key, p);
  return p;
}

export function clearThumbnailMemory() {
  urls.clear();
}

/** Thumbnail URL for a grid cell, or null while loading / not an image. */
export function useThumbnail(path: string, name: string, enabled: boolean, size = 96, version = ""): string | null {
  const [url, setUrl] = useState<string | null>(null);
  useEffect(() => {
    setUrl(null);
    // Connection files are not downloaded just to draw a thumbnail.
    if (!enabled || !isTauriAvailable() || !hasThumbnail(name) || isRemotePath(path)) return;
    let live = true;
    void loadThumbnail(path, size, version).then((u) => live && setUrl(u));
    return () => {
      live = false;
    };
  }, [path, name, enabled, size, version]);
  return url;
}
