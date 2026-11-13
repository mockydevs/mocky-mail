import { useEffect, useRef, useState } from 'react';

export type Loaded<T> =
  | { status: 'loading' }
  | { status: 'ready'; value: T }
  | { status: 'error'; message: string };

type Settled<T> = { resource: string; revision: number } & (
  | { ok: true; value: T }
  | { ok: false; message: string }
);

/**
 * Load `resource`. A new resource (another folder, another message) shows
 * loading and ignores late results for the old one, so fast switching never
 * shows the wrong data. A new `revision` of the same resource (after an
 * action) keeps showing the current value until the fresh one arrives.
 */
export function useLoad<T>(resource: string, revision: number, load: () => Promise<T>): Loaded<T> {
  const loadRef = useRef(load);
  const [settled, setSettled] = useState<Settled<T> | null>(null);

  useEffect(() => {
    loadRef.current = load;
  });

  useEffect(() => {
    let active = true;
    loadRef.current().then(
      (value) => {
        if (active) setSettled({ resource, revision, ok: true, value });
      },
      (error: unknown) => {
        if (active) {
          const message = error instanceof Error ? error.message : 'Something went wrong';
          setSettled({ resource, revision, ok: false, message });
        }
      },
    );
    return () => {
      active = false;
    };
  }, [resource, revision]);

  if (settled?.resource !== resource) return { status: 'loading' };
  return settled.ok ? { status: 'ready', value: settled.value } : { status: 'error', message: settled.message };
}
