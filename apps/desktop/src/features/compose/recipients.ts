export interface ParsedRecipients {
  valid: string[];
  invalid: string[];
}

/**
 * A pragmatic address check for the compose form: one "@", a non-empty local
 * part, a dotted domain, no spaces or angle brackets. The server is the final
 * judge; this only catches typos before queueing.
 */
export function isPlausibleAddress(address: string): boolean {
  return /^[^\s@<>(),;:"[\]]+@[^\s@<>(),;:"[\]]+\.[^\s@<>(),;:"[\]]{2,}$/u.test(address);
}

/** Split "a@x.example, b@y.example; c" into valid and invalid addresses, de-duplicated. */
export function parseRecipients(input: string): ParsedRecipients {
  const seen = new Set<string>();
  const valid: string[] = [];
  const invalid: string[] = [];
  for (const raw of input.split(/[,;\n]/)) {
    const address = raw.trim();
    if (!address) continue;
    const key = address.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    (isPlausibleAddress(address) ? valid : invalid).push(address);
  }
  return { valid, invalid };
}
