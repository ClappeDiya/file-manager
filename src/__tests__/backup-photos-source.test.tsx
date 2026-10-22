/**
 * Photos library sources: detected libraries, manual paths as photos://
 * URIs matching the Rust parser, snake_case options and the album picker.
 */
import { beforeEach, describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { useState } from "react";

const mockInvoke = vi.fn();
vi.mock("@tauri-apps/api/core", () => ({ invoke: (...a: unknown[]) => mockInvoke(...a) }));

import { PhotosSourceSection } from "@/components/backup/backup-photos-source";
import { isPhotosSource, photosLibraryPath, photosSourceUri } from "@/lib/photos-source";
import { newBackupJob, type BackupJob, type PhotosLibraryCandidate, type PhotosLibraryInfo, type PhotosOptions } from "@/stores/backup-store";

const SYSTEM: PhotosLibraryCandidate = {
  path: "/Users/me/Pictures/Photos Library.photoslibrary",
  name: "Photos Library",
  source: "photos:///Users/me/Pictures/Photos Library.photoslibrary",
  has_database: true,
  is_default: true,
};

const INFO: PhotosLibraryInfo = { path: SYSTEM.path, assets: 120, hidden: 3, trashed: 2, albums: ["Trip", "Family"], warnings: [] };

const changes = vi.fn<(sources: string[], photos: PhotosOptions) => void>();
/** The job as last changed by the section. */
const latest = () => {
  const [sources, photos] = changes.mock.calls[changes.mock.calls.length - 1];
  return { sources, options: { photos } };
};

function Harness({ initial }: { initial: BackupJob }) {
  const [job, setJob] = useState(initial);
  return (
    <PhotosSourceSection
      job={job}
      onChange={(sources, photos) => {
        changes(sources, photos);
        setJob((j) => ({ ...j, sources, options: { ...j.options, photos } }));
      }}
    />
  );
}

beforeEach(() => {
  mockInvoke.mockReset();
  changes.mockReset();
  (window as unknown as Record<string, unknown>).__TAURI__ = {};
  (window as unknown as Record<string, unknown>).__TAURI_INTERNALS__ = {};
  mockInvoke.mockImplementation(async (cmd: string) => {
    if (cmd === "backup_photos_libraries") return [SYSTEM];
    if (cmd === "backup_photos_library_info") return INFO;
    return null;
  });
});

describe("photos source helpers", () => {
  it("builds the same URIs as the Rust side", () => {
    expect(photosSourceUri("/Users/me/Pictures/L.photoslibrary/")).toBe("photos:///Users/me/Pictures/L.photoslibrary");
    expect(photosSourceUri("C:\\Pics\\L.photoslibrary")).toBe("photos:///C:\\Pics\\L.photoslibrary");
    expect(photosLibraryPath("photos:///C:\\Pics\\L.photoslibrary")).toBe("C:\\Pics\\L.photoslibrary");
    expect(photosLibraryPath(SYSTEM.source)).toBe(SYSTEM.path);
    expect(isPhotosSource(" photos:///x")).toBe(true);
    expect(isPhotosSource("/Users/me/Pictures")).toBe(false);
  });

  it("new jobs carry default photos options in Rust's shape", () => {
    expect(newBackupJob().options.photos).toEqual({ include_hidden: false, include_trashed: false, albums: [] });
  });
});

describe("photos source section", () => {
  it("adds a detected library and edits its options", async () => {
    render(<Harness initial={{ ...newBackupJob(), sources: ["/home/me/Docs"] }} />);
    const add = await screen.findByRole("button", { name: "Add" });
    expect(screen.getByText("(system library)")).toBeInTheDocument();
    fireEvent.click(add);
    expect(latest().sources).toEqual(["/home/me/Docs", SYSTEM.source]);
    expect(screen.getByRole("button", { name: "Added" })).toBeDisabled();

    fireEvent.click(screen.getByLabelText("Include hidden photos"));
    expect(latest().options.photos).toEqual({ include_hidden: true, include_trashed: false, albums: [] });

    fireEvent.click(screen.getByTestId("photos-load-albums"));
    await screen.findByTestId("photos-info");
    expect(mockInvoke).toHaveBeenCalledWith("backup_photos_library_info", { path: SYSTEM.source, tempDir: null });
    expect(screen.getByText(/120 items \(3 hidden, 2 recently deleted\), 2 albums/)).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Family" }));
    expect(latest().options.photos.albums).toEqual(["Family"]);
    expect((screen.getByTestId("photos-albums") as HTMLTextAreaElement).value).toBe("Family");
    fireEvent.change(screen.getByTestId("photos-albums"), { target: { value: "Family\n  Trip \n" } });
    expect(latest().options.photos.albums).toEqual(["Family", "Trip"]);

    fireEvent.click(screen.getByRole("button", { name: "Remove" }));
    expect(latest().sources).toEqual(["/home/me/Docs"]);
    expect(screen.queryByTestId("photos-options")).toBeNull();
  });

  it("adds a typed library path as a photos:// source", async () => {
    mockInvoke.mockImplementation(async () => []);
    render(<Harness initial={newBackupJob()} />);
    await waitFor(() => expect(mockInvoke).toHaveBeenCalledWith("backup_photos_libraries", undefined));
    expect(screen.queryByTestId("photos-candidates")).toBeNull();
    expect(screen.getByTestId("photos-add")).toBeDisabled();
    fireEvent.change(screen.getByTestId("photos-path"), { target: { value: "/Volumes/Ext/Old.photoslibrary" } });
    fireEvent.click(screen.getByTestId("photos-add"));
    expect(latest().sources).toEqual(["photos:///Volumes/Ext/Old.photoslibrary"]);
    expect(screen.getByLabelText(/Include Recently Deleted/)).not.toBeChecked();
  });
});
