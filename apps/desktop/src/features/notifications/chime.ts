/**
 * The new-mail sound: two short, soft rising notes made with Web Audio, so
 * no sound file ships with the app. Several accounts often report new mail
 * within moments of each other, so it plays at most once every few seconds.
 */

/** G5 then C6: a gentle rising "toot toot". */
export const CHIME_NOTES = [784, 1047] as const;
const NOTE_SECONDS = 0.14;
const GAP_SECONDS = 0.05;
const PEAK_GAIN = 0.12;
export const CHIME_MIN_INTERVAL_MS = 3000;

let context: AudioContext | null = null;
let lastPlayedAt = Number.NEGATIVE_INFINITY;

function audio(): AudioContext | null {
  if (typeof AudioContext === 'undefined') return null;
  try {
    context ??= new AudioContext();
  } catch {
    // No usable audio device (common on Linux without a sound server).
    return null;
  }
  return context;
}

function note(ctx: AudioContext, frequency: number, start: number): void {
  const osc = ctx.createOscillator();
  const gain = ctx.createGain();
  osc.type = 'sine';
  osc.frequency.setValueAtTime(frequency, start);
  // Quick fade in, smooth fade out: no clicks, nothing harsh.
  gain.gain.setValueAtTime(0.0001, start);
  gain.gain.exponentialRampToValueAtTime(PEAK_GAIN, start + 0.012);
  gain.gain.exponentialRampToValueAtTime(0.0001, start + NOTE_SECONDS);
  osc.connect(gain);
  gain.connect(ctx.destination);
  osc.start(start);
  osc.stop(start + NOTE_SECONDS + 0.02);
}

/**
 * Play the chime. Returns false when it was skipped (played moments ago, or
 * the system has no audio). `force` is for the Settings preview button.
 */
export function playChime({ now = Date.now(), force = false }: { now?: number; force?: boolean } = {}): boolean {
  if (!force && now - lastPlayedAt < CHIME_MIN_INTERVAL_MS) return false;
  const ctx = audio();
  if (!ctx) return false;
  lastPlayedAt = now;
  // A context created before any click starts suspended; resuming is allowed in the desktop app.
  if (ctx.state === 'suspended') void ctx.resume().catch(() => undefined);
  const start = ctx.currentTime + 0.02;
  CHIME_NOTES.forEach((frequency, i) => {
    note(ctx, frequency, start + i * (NOTE_SECONDS + GAP_SECONDS));
  });
  return true;
}

/** Tests only: forget the shared audio context and the last play time. */
export function resetChimeForTests(): void {
  context = null;
  lastPlayedAt = Number.NEGATIVE_INFINITY;
}
