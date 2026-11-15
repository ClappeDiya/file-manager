


export interface FileEntry {
  id: string;
  name: string;
  type: "file" | "folder";
  size: number;
  modified: string;
  permissions: string;
}


/**
 * Generate demo file entries for testing.
 */
export function generateDemoFiles(count: number): FileEntry[] {
  const extensions = [".ts", ".tsx", ".json", ".css", ".md", ".txt", ".png", ".svg"];
  const folders = ["src", "components", "hooks", "lib", "pages", "assets", "styles", "utils"];

  return Array.from({ length: count }, (_, i) => {
    const isFolder = i < Math.floor(count * 0.2); // 20% folders
    const ext = extensions[i % extensions.length];
    const folder = folders[i % folders.length];

    return {
      id: `file-${i}`,
      name: isFolder ? folder : `file-${i}${ext}`,
      type: isFolder ? ("folder" as const) : ("file" as const),
      size: isFolder ? 0 : Math.floor(Math.random() * 1024 * 1024),
      modified: new Date(
        Date.now() - Math.floor(Math.random() * 30 * 24 * 60 * 60 * 1000),
      ).toISOString(),
      permissions: isFolder ? "drwxr-xr-x" : "-rw-r--r--",
    };
  });
}
