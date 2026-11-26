import { MailApiProvider } from '../api/MailApiContext';
import type { MailApi } from '../api/types';
import { MessageWindow } from '../features/reader/MessageWindow';
import { Shell } from './Shell';

/**
 * The whole app, or (with `messageId`) one message in a window of its own,
 * as opened by "Open in new window" (`?message=<id>`).
 */
export function App({ api, now, messageId = null }: { api: MailApi; now?: () => number; messageId?: string | null }) {
  const body = messageId ? (
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
