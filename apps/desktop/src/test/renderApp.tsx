import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';

import { MOCK_NOW } from '../api/mock/mockData';
import { createMockMailApi, type MockMailApi, type MockOptions } from '../api/mock/mockMailApi';
import { App } from '../app/App';

export function renderApp(options: MockOptions = {}, api: MockMailApi = createMockMailApi(options)) {
  const user = userEvent.setup();
  render(<App api={api} now={() => MOCK_NOW} />);
  return { user, api };
}

export const unified = () => within(screen.getByRole('list', { name: 'All accounts' }));
export const accountFolders = (name: string) => within(screen.getByRole('list', { name }));
/** The header row that opens or closes an account's folders. */
export const accountToggle = (name: string) =>
  within(screen.getByRole('navigation', { name: 'Mailboxes' })).getByRole('button', {
    name: new RegExp(`^${name} \\(`),
  });
export const queryAccountToggle = (name: string) =>
  within(screen.getByRole('navigation', { name: 'Mailboxes' })).queryByRole('button', {
    name: new RegExp(`^${name} \\(`),
  });
export const reader = () => screen.getByRole('article', { name: 'Reading pane' });
export async function rows(title = 'Inbox') {
  return within(await screen.findByRole('list', { name: `${title} messages` }));
}

export function deferred<T = void>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
