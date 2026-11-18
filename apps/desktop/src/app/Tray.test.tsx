import { act, screen, within } from '@testing-library/react';

import { MOCK_SETTINGS } from '../api/mock/mockData';
import { renderApp, rows } from '../test/renderApp';

describe('tray and start at login', () => {
  it("opens a new message from the tray icon's menu", async () => {
    const { api } = renderApp();
    await rows();
    act(() => {
      api.emit('mail://compose', {});
    });
    expect(await screen.findByRole('form', { name: 'New message' })).toBeInTheDocument();
  });

  it('offers the tray and start at login on a computer', async () => {
    const { user, api } = renderApp();
    await rows();
    await user.click(screen.getByRole('button', { name: 'Settings' }));
    const group = within(await screen.findByRole('group', { name: 'On this computer' }));
    expect(group.getByRole('checkbox', { name: 'Keep running in the tray when the window is closed' })).toBeChecked();
    const login = group.getByRole('checkbox', { name: 'Open Mocky Mail when I sign in to this computer' });
    expect(login).not.toBeChecked();
    await user.click(login);
    expect((await api.getSettings()).startAtLogin).toBe(true);
  });

  it('has none of this on a phone', async () => {
    const phone = renderApp({ settings: { ...MOCK_SETTINGS, device: { tray: false, startAtLogin: false } } });
    await rows();
    await phone.user.click(screen.getByRole('button', { name: 'Settings' }));
    await screen.findByRole('group', { name: 'Notifications' });
    expect(screen.queryByRole('group', { name: 'On this computer' })).not.toBeInTheDocument();
  });
});
