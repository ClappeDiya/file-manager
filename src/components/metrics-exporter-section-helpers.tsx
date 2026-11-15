


/** Port validation shared with the backend (1024–65535). */
export function portProblem(raw: string): string | null {
  const n = Number(raw);
  if (!/^\d+$/.test(raw.trim()) || n < 1024 || n > 65535) return "Use a port from 1024 to 65535.";
  return null;
}
