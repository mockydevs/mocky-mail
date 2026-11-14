import { useEffect, useRef } from 'react';

import type { MailApi, MailEventName, MailEvents } from '../api/types';

/** Subscribe to a backend event for the component's lifetime. */
export function useMailEvent<E extends MailEventName>(
  api: MailApi,
  event: E,
  handler: (payload: MailEvents[E]) => void,
): void {
  const ref = useRef(handler);
  useEffect(() => {
    ref.current = handler;
  });
  useEffect(
    () =>
      api.on(event, (payload) => {
        ref.current(payload);
      }),
    [api, event],
  );
}
