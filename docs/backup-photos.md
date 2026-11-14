# Backing up a macOS Photos library

A backup job can export the originals of a Photos library as a plain folder tree, together with the album structure, instead of copying the opaque `.photoslibrary` bundle.

## Adding a library

In the job editor, open **Sources → Photos libraries**:

- On macOS, libraries in `~/Pictures` (including the system library, `Photos Library.photoslibrary`) are listed. Click **Add**.
- On any system, type or choose a library path and click **Add Photos library**. Use this for libraries on external drives or copied from a Mac.

The job then lists a source like `photos:///Users/me/Pictures/Photos Library.photoslibrary`. From the command line:

```bash
ufop backup run --dest /Volumes/Backup/photos --source "photos:///Users/me/Pictures/Photos Library.photoslibrary" \
  --photos-include-hidden --photos-album "Trip 2024"
```

UFOP needs **Full Disk Access** (System Settings › Privacy & Security) to read the library.

## Options

| Option | CLI flag | Default |
|---|---|---|
| Include hidden photos | `--photos-include-hidden` | off |
| Include Recently Deleted | `--photos-include-trashed` | off |
| Only these albums (titles, case-insensitive) | `--photos-album <title>` (repeatable) | all assets |

The options apply to every Photos source of the job. **Show albums** reads the library and lists its albums so you can pick them. Ordinary filters also apply, matched against the export paths below.

## What is backed up

Each library becomes a folder next to it, `<library name> (Photos export)`:

```text
Photos Library (Photos export)/
  albums.json
  Photos/2024/03/IMG_0001.HEIC
  Photos/2024/03/IMG_0001 (1A2B3C4D).HEIC
  Photos/Undated/…
```

- Files are grouped by the capture date (in the time zone the photo was taken, when Photos recorded it) and keep their original import name.
- Two assets with the same name in the same month: the later one gets the first eight characters of its UUID added. Names stay the same between runs, unless an older photo with the same name is added later.
- Originals come from `originals/` (Photos 5 and later) or `Masters/` (libraries upgraded from older versions). Edited versions, previews and Photos' own database are not backed up.
- Unchanged originals are recognised by size and timestamp, so later runs only upload new photos.

`albums.json` lists every user album with the paths of its assets, and for each asset its UUID, original file name, capture date, favourite, hidden and recently-deleted flags, kind (photo or video) and albums. Smart albums, shared albums and albums in Recently Deleted are not included.

## Restoring

A restore writes the plain folder tree, never into a library. Restoring to the original location creates the `(Photos export)` folder next to the library. To bring the photos back into Photos, import the restored `Photos` folder and use `albums.json` to recreate albums.

## Warnings and limits

- Originals that are not on the Mac are skipped with a warning naming the file and its UUID. This happens with **Optimize Mac Storage** in iCloud Photos; choose **Download Originals to this Mac** before backing up.
- Photos keeps its database locked, so UFOP copies `database/Photos.sqlite` (with its `-wal` and `-shm` files) to the temporary folder and reads the copy. Allow room for it; large libraries have databases of several hundred MB.
- The database layout changes between macOS versions. Missing tables or columns give fewer details and a warning, not a failed backup. Libraries without `database/Photos.sqlite` (Photos 4 and earlier, never opened in a newer Photos) are refused.
- Referenced files (originals left outside the library) are reported as missing.
- The reader was tested against synthetic libraries built with the real table and column names (`ZASSET`, `ZGENERICALBUM`, `ZADDITIONALASSETATTRIBUTES`, `Z_<n>ASSETS`), not against libraries from every macOS release.
