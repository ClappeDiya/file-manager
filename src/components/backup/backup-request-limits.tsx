/**
 * Request-rate limits and listing parallelism of a destination
 * (`RequestLimits` in ufop-core): stay under provider throttling or
 * per-request billing; 0 = unlimited.
 */
import { inputCls } from "@/lib/backup-helpers";
import type { RequestLimits } from "@/stores/backup-store";
import { Field, Section } from "./backup-ui";

const FIELDS: [keyof RequestLimits, string, boolean][] = [
  ["list_per_sec", "Listings per second", false],
  ["get_per_sec", "Downloads per second", false],
  ["put_per_sec", "Uploads per second", false],
  ["delete_per_sec", "Deletes per second", false],
  ["max_concurrent_requests", "Requests at once", true],
  ["list_parallelism", "Folders listed at once (sharded layouts)", true],
];

export function RequestLimitsEditor({ value, onChange }: { value: RequestLimits; onChange: (v: RequestLimits) => void }) {
  return (
    <Section title="Request limits" hint="Per destination; 0 = unlimited. Each retry counts as a request.">
      <div className="grid grid-cols-2 sm:grid-cols-3 gap-2" data-testid="request-limits">
        {FIELDS.map(([k, label, whole]) => (
          <Field key={k} label={label}>
            <input
              className={inputCls}
              type="number"
              min={0}
              step={whole ? 1 : 0.1}
              value={value[k]}
              onChange={(e) => {
                const n = Math.max(0, Number(e.target.value) || 0);
                onChange({ ...value, [k]: whole ? Math.min(k === "list_parallelism" ? 64 : 10_000, Math.floor(n)) : n });
              }}
              data-testid={`limit-${k}`}
            />
          </Field>
        ))}
      </div>
    </Section>
  );
}
