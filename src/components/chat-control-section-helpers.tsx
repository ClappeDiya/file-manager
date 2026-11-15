


/** Ids as typed: separated by commas, spaces or new lines. */
export function parseIds(text: string): string[] {
  return [...new Set(text.split(/[\s,;]+/).map((s) => s.trim()).filter(Boolean))];
}
