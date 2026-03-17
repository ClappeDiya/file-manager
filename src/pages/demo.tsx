import { useMemo } from "react";
import { Link } from "react-router-dom";
import { DataGrid, generateDemoFiles } from "@/components/data-grid";
import { VirtualList } from "@/components/virtual-list";
import { AccessibleList, ListItem } from "@/components/accessible-list";
import { ThemeSwitcher } from "@/components/theme-switcher";
import { Button, Badge, Separator, Tabs, TabsList, TabsTrigger, TabsContent } from "@ufop/ui-components";
import { ArrowLeft } from "lucide-react";

/**
 * Demo page showcasing TanStack Table, TanStack Virtual, and React Aria components.
 */
export function DemoPage() {
  // Generate demo data
  const smallDataset = useMemo(() => generateDemoFiles(25), []);
  const largeDataset = useMemo(() => generateDemoFiles(10000), []);

  // Sample file names for accessible list
  const sampleFiles = [
    "package.json",
    "tsconfig.json",
    "vite.config.ts",
    "index.html",
    "README.md",
    "src/main.tsx",
    "src/App.tsx",
    "src/styles/globals.css",
  ];

  return (
    <div className="flex h-full flex-col">
      {/* Toolbar */}
      <header
        className="flex items-center justify-between border-b border-border bg-toolbar-bg px-4"
        style={{ height: "var(--toolbar-height)" }}
        role="toolbar"
        aria-label="Demo page toolbar"
      >
        <div className="flex items-center gap-3">
          <Link to="/" aria-label="Back to home">
            <Button variant="ghost" size="icon" aria-label="Navigate back to home">
              <ArrowLeft className="h-4 w-4" aria-hidden="true" />
            </Button>
          </Link>
          <h1 className="text-[length:var(--font-size-md)] font-semibold text-[color:var(--color-text)]">
            Component Demos
          </h1>
        </div>
        <ThemeSwitcher />
      </header>

      {/* Main content */}
      <main className="flex-1 overflow-auto p-6">
        <div className="mx-auto max-w-5xl space-y-8">
          <Tabs defaultValue="table">
            <TabsList aria-label="Demo sections">
              <TabsTrigger value="table">TanStack Table</TabsTrigger>
              <TabsTrigger value="virtual">TanStack Virtual</TabsTrigger>
              <TabsTrigger value="aria">React Aria</TabsTrigger>
            </TabsList>

            {/* TanStack Table Demo */}
            <TabsContent value="table">
              <div className="space-y-4 pt-2">
                <div>
                  <h2 className="text-[length:var(--font-size-xl)] font-semibold text-[color:var(--color-text)]">
                    TanStack Table Data Grid
                  </h2>
                  <p className="text-[color:var(--color-text-secondary)] mt-1">
                    Sortable columns with click or keyboard. Filter by typing in the search box.
                  </p>
                </div>
                <DataGrid data={smallDataset} />
              </div>
            </TabsContent>

            {/* TanStack Virtual Demo */}
            <TabsContent value="virtual">
              <div className="space-y-4 pt-2">
                <div>
                  <h2 className="text-[length:var(--font-size-xl)] font-semibold text-[color:var(--color-text)]">
                    TanStack Virtual List
                  </h2>
                  <p className="text-[color:var(--color-text-secondary)] mt-1">
                    Rendering{" "}
                    <Badge variant="info">
                      {largeDataset.length.toLocaleString()}
                    </Badge>{" "}
                    items with virtualization. Use Arrow keys, Page Up/Down,
                    Home/End to navigate.
                  </p>
                </div>
                <VirtualList items={largeDataset} />
              </div>
            </TabsContent>

            {/* React Aria Demo */}
            <TabsContent value="aria">
              <div className="space-y-4 pt-2">
                <div>
                  <h2 className="text-[length:var(--font-size-xl)] font-semibold text-[color:var(--color-text)]">
                    React Aria Accessible List
                  </h2>
                  <p className="text-[color:var(--color-text-secondary)] mt-1">
                    Full keyboard navigation with React Aria hooks. Try Arrow
                    keys, Home, End, and type-ahead search.
                  </p>
                </div>

                <AccessibleList
                  label="Project Files"
                  selectionMode="multiple"
                  aria-label="Project files list"
                >
                  {sampleFiles.map((file) => (
                    <ListItem key={file} textValue={file}>
                      <span className="flex items-center gap-2">
                        <span className="text-[color:var(--color-text)]">{file}</span>
                      </span>
                    </ListItem>
                  ))}
                </AccessibleList>

                <Separator />

                <div className="rounded-md border border-border bg-[color:var(--color-bg-secondary)] p-4">
                  <h3 className="text-[length:var(--font-size-base)] font-medium text-[color:var(--color-text)] mb-2">
                    Accessibility Features
                  </h3>
                  <ul className="space-y-1 text-[length:var(--font-size-sm)] text-[color:var(--color-text-secondary)]">
                    <li>Arrow Up/Down: Navigate between items</li>
                    <li>Home/End: Jump to first/last item</li>
                    <li>Space: Toggle selection (multi-select mode)</li>
                    <li>Type-ahead: Type to jump to matching items</li>
                    <li>Tab: Move focus in/out of the list</li>
                    <li>All items have proper ARIA roles and labels</li>
                    <li>Focus ring visible on keyboard navigation</li>
                  </ul>
                </div>
              </div>
            </TabsContent>
          </Tabs>
        </div>
      </main>

      {/* Status Bar */}
      <footer
        className="flex items-center justify-between border-t border-border bg-[color:var(--color-bg-secondary)] px-4"
        style={{ height: "var(--status-bar-height)" }}
        role="contentinfo"
        aria-label="Status bar"
      >
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          Demo Mode
        </span>
        <span className="text-[length:var(--font-size-xs)] text-[color:var(--color-text-tertiary)]">
          UFOP v0.1.0
        </span>
      </footer>
    </div>
  );
}
