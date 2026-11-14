/**
 * Job-editor section for macOS Photos library sources (`photos://<library>`).
 * The backend expands each library into its originals under
 * `<library name> (Photos export)/Photos/<YYYY>/<MM>/…` plus `albums.json`.
 */
import { useEffect, useState } from "react";
import { tauriInvoke } from "@/hooks/use-tauri";
import {
  errorText,
  type BackupJob,
  type PhotosLibraryCandidate,
  type PhotosLibraryInfo,
  type PhotosOptions,
} from "@/stores/backup-store";
import { inputCls } from "@/lib/backup-helpers";
import { pickFiles, pickFolder } from "@/lib/folder-picker";
import { isPhotosSource, photosLibraryPath, photosSourceUri } from "@/lib/photos-source";
import { Banner, Btn, Check, Field, Section } from "./backup-ui";

const DEFAULT_PHOTOS: PhotosOptions = { include_hidden: false, include_trashed: false, albums: [] };

const isMac = () => typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);

export function PhotosSourceSection({
  job,
  onChange,
}: {
  job: BackupJob;
  onChange: (sources: string[], photos: PhotosOptions) => void;
}) {
  const photos = job.options.photos ?? DEFAULT_PHOTOS;
  const [candidates, setCandidates] = useState<PhotosLibraryCandidate[]>([]);
  const [path, setPath] = useState("");
  const [info, setInfo] = useState<PhotosLibraryInfo[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [albumText, setAlbumText] = useState(photos.albums.join("\n"));

  useEffect(() => {
    tauriInvoke<PhotosLibraryCandidate[]>("backup_photos_libraries", undefined, [])
      .then((c) => setCandidates(c ?? []))
      .catch(() => setCandidates([]));
  }, []);

  const sources = job.sources.filter((s) => s.trim());
  const photoSources = sources.filter(isPhotosSource);
  const setPhotos = (p: Partial<PhotosOptions>) => onChange(job.sources, { ...photos, ...p });
  const add = (source: string) => {
    if (!sources.includes(source)) onChange([...sources, source], photos);
  };
  const remove = (source: string) => onChange(sources.filter((s) => s !== source), photos);

  const choose = async () => {
    // macOS shows a .photoslibrary bundle as a file; elsewhere it is a folder.
    const picked = isMac() ? (await pickFiles("Choose a Photos library"))[0] ?? null : await pickFolder("Choose a Photos library");
    if (picked) setPath(picked);
  };

  const loadAlbums = async () => {
    setBusy(true);
    setError(null);
    try {
      const out: PhotosLibraryInfo[] = [];
      for (const s of photoSources) {
        out.push(await tauriInvoke<PhotosLibraryInfo>("backup_photos_library_info", { path: s, tempDir: job.options.temp_dir }));
      }
      setInfo(out);
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const applyAlbums = (text: string) => {
    setAlbumText(text);
    setPhotos({ albums: text.split("\n").map((a) => a.trim()).filter(Boolean) });
  };
  const toggleAlbum = (title: string) => {
    const has = photos.albums.some((a) => a.toLowerCase() === title.toLowerCase());
    const next = has ? photos.albums.filter((a) => a.toLowerCase() !== title.toLowerCase()) : [...photos.albums, title];
    setAlbumText(next.join("\n"));
    setPhotos({ albums: next });
  };

  return (
    <Section
      title="Photos libraries"
      hint="Back up the original photos and videos of a macOS Photos library as a plain folder tree (Photos/<year>/<month>/…) with an albums.json describing albums, favourites and dates."
    >
      {error && <Banner tone="error" onClose={() => setError(null)}>{error}</Banner>}
      {candidates.length > 0 && (
        <ul className="space-y-1" data-testid="photos-candidates">
          {candidates.map((c) => (
            <li key={c.path} className="flex items-center gap-2 text-xs">
              <span className="flex-1 truncate" title={c.path}>
                <b>{c.name}</b> {c.is_default && <span className="text-zinc-500">(system library)</span>}
                {!c.has_database && <span className="text-amber-600"> — not upgraded; open it in Photos first</span>}
              </span>
              <Btn disabled={sources.includes(c.source) || !c.has_database} onClick={() => add(c.source)}>
                {sources.includes(c.source) ? "Added" : "Add"}
              </Btn>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-1">
        <input
          className={`${inputCls} font-mono`}
          placeholder="/Users/me/Pictures/Photos Library.photoslibrary"
          value={path}
          onChange={(e) => setPath(e.target.value)}
          data-testid="photos-path"
        />
        <Btn onClick={choose}>Choose…</Btn>
        <Btn
          testId="photos-add"
          disabled={!path.trim()}
          onClick={() => {
            add(photosSourceUri(path));
            setPath("");
          }}
        >
          Add Photos library
        </Btn>
      </div>

      {photoSources.length > 0 && (
        <div className="space-y-2" data-testid="photos-options">
          <ul className="text-xs font-mono space-y-0.5">
            {photoSources.map((s) => (
              <li key={s} className="flex items-center gap-2">
                <span className="flex-1 truncate" title={s}>{photosLibraryPath(s)}</span>
                <Btn onClick={() => remove(s)}>Remove</Btn>
              </li>
            ))}
          </ul>
          <Check label="Include hidden photos" checked={photos.include_hidden} onChange={(v) => setPhotos({ include_hidden: v })} />
          <Check
            label="Include Recently Deleted"
            checked={photos.include_trashed}
            onChange={(v) => setPhotos({ include_trashed: v })}
            hint="Off by default: items in Recently Deleted are left out."
          />
          <Field label="Only these albums" hint="One album title per line; empty backs up the whole library. Applies to every Photos source of this job.">
            <textarea
              className={`${inputCls} font-mono`}
              rows={3}
              value={albumText}
              onChange={(e) => applyAlbums(e.target.value)}
              data-testid="photos-albums"
            />
          </Field>
          <Btn disabled={busy} onClick={loadAlbums} testId="photos-load-albums">
            {busy ? "Reading library…" : "Show albums"}
          </Btn>
          {info.map((i) => (
            <div key={i.path} className="text-xs text-zinc-600 dark:text-zinc-400" data-testid="photos-info">
              <div>
                {i.assets} items ({i.hidden} hidden, {i.trashed} recently deleted), {i.albums.length} albums
              </div>
              <div className="flex flex-wrap gap-1 mt-1">
                {i.albums.map((a) => {
                  const on = photos.albums.some((x) => x.toLowerCase() === a.toLowerCase());
                  return (
                    <button
                      key={a}
                      type="button"
                      className={`px-1.5 py-0.5 rounded border text-[11px] ${on ? "bg-blue-500 text-white border-blue-500" : "border-zinc-300 dark:border-zinc-600"}`}
                      onClick={() => toggleAlbum(a)}
                    >
                      {a}
                    </button>
                  );
                })}
              </div>
              {i.warnings.map((w) => (
                <div key={w} className="text-amber-600">{w}</div>
              ))}
            </div>
          ))}
          <p className="text-[11px] text-zinc-500">
            Originals kept only in iCloud ("Optimize Mac Storage") are skipped with a warning. UFOP needs Full Disk Access to read the library.
          </p>
        </div>
      )}
    </Section>
  );
}
