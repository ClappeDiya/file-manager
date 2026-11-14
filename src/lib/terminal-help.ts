/** Types and search of the terminal's command help drawer (Rust `terminal_help_commands`). */

export interface CliOptionHelp {
  flags: string[];
  help: string;
  takes_value: boolean;
}

export interface CliCommandHelp {
  path: string[];
  about: string;
  usage: string;
  has_subcommands: boolean;
  options: CliOptionHelp[];
}

export interface RcloneCommandHelp {
  name: string;
  about: string;
}

export interface TerminalHelp {
  ufop: CliCommandHelp[];
  rclone: RcloneCommandHelp[] | null;
  rclone_version: string | null;
  rclone_note: string | null;
}

export const NO_HELP: TerminalHelp = { ufop: [], rclone: null, rclone_version: null, rclone_note: null };

/** One row of the drawer. */
export interface HelpEntry {
  tool: "ufop" | "rclone";
  /** What the row shows and the text it inserts without the trailing space (`ufop backup run`). */
  command: string;
  about: string;
  /** The command's synopsis, when known. */
  usage: string | null;
  options: CliOptionHelp[];
}

export function entriesOf(help: TerminalHelp): HelpEntry[] {
  const ufop = help.ufop.map<HelpEntry>((c) => ({ tool: "ufop", command: `ufop ${c.path.join(" ")}`, about: c.about, usage: `ufop ${c.usage}`, options: c.options }));
  const rclone = (help.rclone ?? []).map<HelpEntry>((c) => ({ tool: "rclone", command: `rclone ${c.name}`, about: c.about, usage: null, options: [] }));
  return [...ufop, ...rclone];
}

/** Entries matching every word of `query` in the command or its description (ignoring case); all of them for an empty query. */
export function searchEntries(entries: HelpEntry[], query: string): HelpEntry[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length === 0) return entries;
  return entries.filter((e) => {
    const hay = `${e.command} ${e.about}`.toLowerCase();
    return words.every((w) => hay.includes(w));
  });
}

/** What a click types at the prompt: the command and a space to start the arguments. Nothing is run. */
export const insertText = (e: HelpEntry): string => `${e.command} `;
