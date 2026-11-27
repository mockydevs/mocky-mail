import { MailApiProvider } from '../api/MailApiContext';
import type { MailApi } from '../api/types';
import { ComposeWindow } from '../features/compose/ComposeWindow';
import { MessageWindow } from '../features/reader/MessageWindow';
import { Shell } from './Shell';

/**
 * The whole app, or (with `messageId`) one message in a window of its own,
 * as opened by "Open in new window" (`?message=<id>`), or (with
 * `composeKey`) a message being written in a window of its own, as opened
 * by Pop out (`?compose=<key>`).
 */
export function App({
  api,
  now,
  messageId = null,
  composeKey = null,
}: {
  api: MailApi;
  now?: () => number;
  messageId?: string | null;
  composeKey?: string | null;
}) {
  const body = composeKey ? (
    <ComposeWindow composeKey={composeKey} />
  ) : messageId ? (
    now ? (
      <MessageWindow messageId={messageId} now={now} />
    ) : (
      <MessageWindow messageId={messageId} />
    )
  ) : now ? (
    <Shell now={now} />
  ) : (
    <Shell />
  );
  return <MailApiProvider api={api}>{body}</MailApiProvider>;
}
