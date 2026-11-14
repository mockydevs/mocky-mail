import { vi } from 'vitest';

import { CHIME_MIN_INTERVAL_MS, CHIME_NOTES, playChime, resetChimeForTests } from './chime';

/** A stand-in for Web Audio that records the notes played. */
function fakeAudio(state: AudioContextState = 'running') {
  const started: number[] = [];
  const resume = vi.fn(() => Promise.resolve());
  class FakeContext {
    state = state;
    currentTime = 0;
    destination = {};
    resume = resume;
    createOscillator() {
      const osc = {
        type: '',
        frequency: { value: 0, setValueAtTime: (v: number) => (osc.frequency.value = v) },
        connect: vi.fn(),
        start: () => started.push(osc.frequency.value),
        stop: vi.fn(),
      };
      return osc;
    }
    createGain() {
      return { gain: { setValueAtTime: vi.fn(), exponentialRampToValueAtTime: vi.fn() }, connect: vi.fn() };
    }
  }
  vi.stubGlobal('AudioContext', FakeContext);
  return { started, resume };
}

describe('new-mail chime', () => {
  beforeEach(() => {
    resetChimeForTests();
  });
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it('plays two soft rising notes', () => {
    const { started } = fakeAudio();
    expect(playChime({ now: 10_000 })).toBe(true);
    expect(started).toEqual([...CHIME_NOTES]);
    expect(CHIME_NOTES[1]).toBeGreaterThan(CHIME_NOTES[0]);
  });

  it('plays once when several accounts report new mail together', () => {
    const { started } = fakeAudio();
    expect(playChime({ now: 10_000 })).toBe(true);
    expect(playChime({ now: 10_500 })).toBe(false);
    expect(started).toHaveLength(2);
    expect(playChime({ now: 10_000 + CHIME_MIN_INTERVAL_MS })).toBe(true);
    expect(started).toHaveLength(4);
  });

  it('always plays for the Settings preview', () => {
    const { started } = fakeAudio();
    playChime({ now: 10_000 });
    expect(playChime({ now: 10_001, force: true })).toBe(true);
    expect(started).toHaveLength(4);
  });

  it('wakes a suspended audio context', () => {
    const { resume } = fakeAudio('suspended');
    playChime({ now: 10_000 });
    expect(resume).toHaveBeenCalled();
  });

  it('does nothing where there is no audio', () => {
    expect(playChime({ now: 10_000 })).toBe(false);
  });

  it('stays quiet instead of throwing when there is no audio device', () => {
    // `new` on a function that throws behaves like a constructor that fails.
    vi.stubGlobal(
      'AudioContext',
      vi.fn(function noAudioDevice() {
        throw new Error('no audio device');
      }),
    );
    expect(playChime()).toBe(false);
  });
});
