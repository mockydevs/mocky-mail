export interface ParsedRecipients {
  /** Bare addresses, de-duplicated (case-insensitive), in order. */
  valid: string[];
  /** Entries as typed that are not usable addresses. */
  invalid: string[];
}

/**
 * A pragmatic address check: one "@", a non-empty local part, a dotted
 * domain, no spaces or brackets. The backend is the final judge; this only
 * catches typos before queueing.
 */
export function isPlausibleAddress(address: string): boolean {
  return /^[^\s@<>(),;:"[\]]+@[^\s@<>(),;:"[\]]+\.[^\s@<>(),;:"[\]]{2,}$/u.test(address);
}

/** Split on commas, semicolons and new lines that are not inside quotes or angle brackets. */
export function splitRecipients(input: string): string[] {
  const parts: string[] = [];
  let current = '';
  let quoted = false;
  let angled = false;
  for (const ch of input) {
    if (ch === '"' && !angled) quoted = !quoted;
    else if (ch === '<' && !quoted) angled = true;
    else if (ch === '>' && !quoted) angled = false;
    if (!quoted && !angled && (ch === ',' || ch === ';' || ch === '\n')) {
      parts.push(current);
      current = '';
    } else {
      current += ch;
    }
  }
  parts.push(current);
  return parts.map((p) => p.trim()).filter(Boolean);
}

/** The address in `Name <address>` or a bare address; null if malformed. */
export function addressOf(entry: string): string | null {
  const angle = /<([^<>]*)>\s*$/.exec(entry);
  if (angle) {
    const before = entry.slice(0, angle.index);
    if (/[<>]/.test(before)) return null;
    return angle[1]?.trim() ?? null;
  }
  return /[<>"]/.test(entry) ? null : entry.trim();
}

/** Parse "Wanjiru Kamau <w@x.example>, b@y.example" into valid and invalid entries. */
export function parseRecipients(input: string): ParsedRecipients {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const entry of splitRecipients(input)) {
    const address = addressOf(entry);
    if (address === null || !isPlausibleAddress(address)) {
      invalid.push(entry);
      continue;
    }
    const key = address.toLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    valid.push(address);
  }
  return { valid, invalid };
}
