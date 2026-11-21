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
 * Everyone currently wanting the Back button, latest last. Back goes only to
 * the last one, so a layer opened over another (a preview over the message)
 * closes first. One Tauri listener serves them all: while any listener is
 * registered, Android doesn't go back in the web view's history itself.
 */
const layers: { current: () => void }[] = [];
let registration: Promise<PluginListener | null> | null = null;

function addLayer(layer: { current: () => void }): () => void {
  layers.push(layer);
  registration ??= onBackButtonPress(() => {
    layers[layers.length - 1]?.current();
  }).catch(() => null);
  return () => {
    const at = layers.lastIndexOf(layer);
    if (at >= 0) layers.splice(at, 1);
    if (layers.length > 0 || !registration) return;
    const done = registration;
    registration = null;
    void done.then((listener) => listener?.unregister());
  };
}

/**
 * Ties the Android Back button to the app's own layers (menu, compose, open
 * message, attachment preview). While `depth` layers are open, Back calls
 * `onBack` to close the top one; with nothing open the button is left alone,
 * so Android does its usual thing and sends the app to the background. When
 * several components use this, the one that opened last gets Back first.
 */
export function usePhoneBack(depth: number, onBack: () => void): void {
  const back = useRef(onBack);
  useEffect(() => {
    back.current = onBack;
  });
  const open = depth > 0;

  useEffect(() => {
    if (!open || !isTauri()) return;
    return addLayer(back);
  }, [open]);
}
