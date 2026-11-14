/**
 * A form generated from an rclone provider schema (`rclone config
 * providers`). Required and everyday options show in Standard mode; the
 * provider's "advanced" options only in Advanced mode, folded away.
 * Passwords and other secrets use masked inputs; on edit, a stored secret
 * shows as "set" and a blank field keeps it.
 */
import { PasswordInput } from "@/components/password-input";
import { fieldValue, isSecret, optionExamples, visibleOptions, type RcloneOption, type RcloneProvider } from "@/lib/rclone";

const input = "w-full text-sm px-2 py-1.5 rounded border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-900";

/** First paragraph of rclone's help text. */
function shortHelp(help: string): string {
  return help.split(/\n\s*\n/)[0].replace(/\s+/g, " ").trim();
}

function OptionField({
  option,
  value,
  values,
  secretSet,
  onChange,
}: {
  option: RcloneOption;
  value: string;
  values: Record<string, string>;
  secretSet: boolean;
  onChange: (v: string) => void;
}) {
  const id = `rclone-opt-${option.name}`;
  const examples = optionExamples(option, values);
  const label = (
    <label htmlFor={id} className="block text-xs font-medium mb-1">
      {option.name}
      {option.required && <span className="text-red-500"> *</span>}
      {option.advanced && <span className="ml-1 text-[10px] text-gray-400">advanced</span>}
    </label>
  );
  const hint = (
    <p className="text-[10px] text-gray-400 dark:text-gray-500 mt-0.5" title={option.help}>
      {shortHelp(option.help)}
      {isSecret(option) && secretSet && " — stored; leave blank to keep it."}
    </p>
  );
  let field;
  if (option.type === "bool") {
    field = (
      <select id={id} value={value} onChange={(e) => onChange(e.target.value)} className={input}>
        <option value="true">Yes</option>
        <option value="false">No</option>
      </select>
    );
  } else if (isSecret(option)) {
    field = (
      <PasswordInput
        id={id}
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder={secretSet ? "•••••• (unchanged)" : option.required ? "Required" : "Optional"}
        autoComplete="new-password"
        className={input}
      />
    );
  } else if (examples.length > 0 && option.exclusive) {
    field = (
      <select id={id} value={value} onChange={(e) => onChange(e.target.value)} className={input}>
        {!option.required && <option value="">(default{option.default ? `: ${option.default}` : ""})</option>}
        {examples.map((ex) => (
          <option key={ex.value} value={ex.value}>
            {ex.value}
            {ex.help ? ` — ${shortHelp(ex.help)}` : ""}
          </option>
        ))}
      </select>
    );
  } else {
    field = (
      <>
        <input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          list={examples.length ? `${id}-examples` : undefined}
          placeholder={option.default || (option.required ? "Required" : "")}
          className={`${input} ${option.type === "string" ? "" : "font-mono"}`}
        />
        {examples.length > 0 && (
          <datalist id={`${id}-examples`}>
            {examples.map((ex) => (
              <option key={ex.value} value={ex.value}>
                {shortHelp(ex.help)}
              </option>
            ))}
          </datalist>
        )}
      </>
    );
  }
  return (
    <div data-testid={id}>
      {label}
      {field}
      {hint}
    </div>
  );
}

export function RcloneSchemaForm({
  provider,
  values,
  onChange,
  advanced,
  secretsSet = [],
}: {
  provider: RcloneProvider;
  values: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
  /** Show the provider's advanced options (Advanced mode). */
  advanced: boolean;
  /** Secrets already stored (edit). */
  secretsSet?: string[];
}) {
  const options = visibleOptions(provider, values, advanced);
  const basic = options.filter((o) => !o.advanced);
  const extra = options.filter((o) => o.advanced);
  const field = (o: RcloneOption) => (
    <OptionField
      key={o.name}
      option={o}
      value={fieldValue(o, values)}
      values={values}
      secretSet={secretsSet.includes(o.name)}
      onChange={(v) => onChange({ ...values, [o.name]: v })}
    />
  );
  return (
    <div className="space-y-2" data-testid="rclone-schema-form">
      {basic.length === 0 && <p className="text-xs text-gray-500">This backend needs no settings.</p>}
      {basic.map(field)}
      {extra.length > 0 && (
        <details className="rounded border border-gray-200 dark:border-gray-700 p-2" data-testid="rclone-advanced-options">
          <summary className="text-xs font-medium cursor-pointer">Advanced options ({extra.length})</summary>
          <div className="space-y-2 mt-2">{extra.map(field)}</div>
        </details>
      )}
    </div>
  );
}
