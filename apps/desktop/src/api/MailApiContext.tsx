import { createContext, use, type ReactNode } from 'react';

import type { MailApi } from './types';

const MailApiContext = createContext<MailApi | null>(null);

export function MailApiProvider({ api, children }: { api: MailApi; children: ReactNode }) {
  return <MailApiContext value={api}>{children}</MailApiContext>;
}

export function useMailApi(): MailApi {
  const api = use(MailApiContext);
  if (!api) throw new Error('useMailApi must be used inside MailApiProvider');
  return api;
}
