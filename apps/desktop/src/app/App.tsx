import { MailApiProvider } from '../api/MailApiContext';
import type { MailApi } from '../api/types';
import { Shell } from './Shell';

export function App({ api, now }: { api: MailApi; now?: () => number }) {
  return <MailApiProvider api={api}>{now ? <Shell now={now} /> : <Shell />}</MailApiProvider>;
}
