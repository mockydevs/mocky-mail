import { onBackButtonPress } from '@tauri-apps/api/app';
import { isTauri, type PluginListener } from '@tauri-apps/api/core';
import { useEffect, useRef, useSyncExternalStore } from 'react';

/** Phone-sized screens get the one-pane layout. */
const PHONE_QUERY = '(max-width: 640px)';

function media(): MediaQueryList | null {
  return typeof window !== 'undefined' && typeof window.matchMedia === 'function' ? window.matchMedia(PHONE_QUERY) : null;
}

/** Whether the window is phone-sized; follows rotation and resizing. */
export function usePhone(): boolean {
  return useSyncExternalStore(
    (changed) => {
      const list = media();
      list?.addEventListener('change', changed);
      return () => list?.removeEventListener('change', changed);
    },
    () => media()?.matches ?? false,
  );
}

/**
 * Ties the Android Back button to the app's own layers (menu, compose, open
 * message). While `depth` layers are open, Back calls `onBack` to close the
 * top one; with nothing open the button is left alone, so Android does its
 * usual thing and sends the app to the background.
 */
export function usePhoneBack(depth: number, onBack: () => void): void {
  const back = useRef(onBack);
  useEffect(() => {
    back.current = onBack;
  });
  const open = depth > 0;

  useEffect(() => {
    if (!open || !isTauri()) return;
    let listener: PluginListener | null = null;
    let cancelled = false;
    onBackButtonPress(() => {
      back.current();
    }).then(
      (registered) => {
        if (cancelled) void registered.unregister();
        else listener = registered;
      },
      () => undefined,
    );
    return () => {
      cancelled = true;
      void listener?.unregister();
    };
  }, [open]);
}
