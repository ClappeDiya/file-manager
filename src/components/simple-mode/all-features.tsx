import { useState } from "react";
import { ArrowRight, Search } from "lucide-react";
import {
  PLATFORM_FEATURES,
  type FeatureGroup,
  type FeatureTarget,
} from "./feature-catalog";

export function AllFeatures({
  group,
  onOpen,
}: {
  group?: FeatureGroup;
  onOpen: (target: FeatureTarget) => void;
}) {
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<FeatureGroup | "all">(group ?? "all");
  const groups = [...new Set(PLATFORM_FEATURES.map((f) => f.group))];
  const q = query.trim().toLocaleLowerCase();
  const features = PLATFORM_FEATURES.filter(
    (f) =>
      (filter === "all" || f.group === filter) &&
      `${f.label} ${f.description} ${f.group}`.toLocaleLowerCase().includes(q),
  );
  return (
    <section
      className="mx-auto max-w-5xl p-4 sm:p-6 space-y-5"
      aria-labelledby="features-heading"
    >
      <div>
        <h2 id="features-heading" className="text-xl font-semibold">
          All features
        </h2>
        <p className="mt-1 text-sm text-[color:var(--color-text-secondary)]">
          Open a group or search for a tool. Every group is available in Basic
          and Advanced layouts.
        </p>
      </div>
      <div className="grid gap-3 sm:grid-cols-[minmax(0,1fr)_auto]">
        <div className="flex-1 min-w-0">
          <label htmlFor="feature-search" className="block text-sm mb-1">
            Find a feature
          </label>
          <div className="relative">
            <Search
              aria-hidden="true"
              className="absolute left-3 top-3 h-4 w-4"
            />
            <input
              id="feature-search"
              type="search"
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Try restore, rclone, encryption…"
              className="w-full min-w-0 rounded-md border border-[var(--color-border)] bg-[var(--color-input-bg)] pl-9 pr-3 py-2 text-sm"
            />
          </div>
        </div>
        <div>
          <label htmlFor="feature-group" className="block text-sm mb-1">
            Group
          </label>
          <select
            id="feature-group"
            value={filter}
            onChange={(e) => setFilter(e.target.value as FeatureGroup | "all")}
            className="w-full rounded-md border border-[var(--color-border)] bg-[var(--color-bg)] px-3 py-2 text-sm"
          >
            <option value="all">All groups</option>
            {groups.map((g) => (
              <option key={g}>{g}</option>
            ))}
          </select>
        </div>
      </div>
      <p
        role="status"
        className="text-xs text-[color:var(--color-text-secondary)]"
      >
        {features.length} features{q ? ` matching “${query.trim()}”` : ""}
      </p>
      {features.length === 0 && (
        <p>No matching features. Try another word or choose All groups.</p>
      )}
      {groups.map((g) => {
        const items = features.filter((f) => f.group === g);
        return (
          items.length > 0 && (
            <details
              key={g}
              open={filter !== "all" || !!q}
              className="rounded-lg border border-[var(--color-border)] p-3"
            >
              <summary className="cursor-pointer font-semibold">
                {g}{" "}
                <span className="text-xs font-normal text-[color:var(--color-text-secondary)]">
                  ({items.length})
                </span>
              </summary>
              <div className="mt-3 grid gap-2 sm:grid-cols-2">
                {items.map((f) => (
                  <button
                    key={f.id}
                    type="button"
                    onClick={() => onOpen(f.target)}
                    className="group rounded-lg border border-[var(--color-border)] bg-[var(--color-bg-elevated)] p-3 text-left hover:bg-[var(--color-hover-bg)] focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--color-focus-ring)]"
                  >
                    <span className="flex items-center justify-between gap-2 font-medium text-sm">
                      {f.label}
                      <ArrowRight
                        className="h-4 w-4 shrink-0"
                        aria-hidden="true"
                      />
                    </span>
                    <span className="mt-1 block text-xs text-[color:var(--color-text-secondary)]">
                      {f.description}
                    </span>
                    {f.selection && (
                      <span className="block mt-2 text-xs font-medium">
                        Select a file in Files first
                      </span>
                    )}
                  </button>
                ))}
              </div>
            </details>
          )
        );
      })}
    </section>
  );
}
