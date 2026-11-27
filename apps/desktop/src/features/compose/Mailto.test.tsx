import { act, screen, within } from '@testing-library/react';

import { createMockMailApi } from '../../api/mock/mockMailApi';
import { messageText } from '../../test/editor';
import { renderApp, rows } from '../../test/renderApp';
import { preloadEditor } from './editor/loader';

// Compose's editor is a chunk of its own: have it here before the first test.
beforeAll(async () => {
  await preloadEditor();
});

describe('mailto: links', () => {
  it('open a filled-in new message when the app starts with one', async () => {
    const api = createMockMailApi();
    api.launchWith('mailto:info@mocky.co.ke?subject=Website%20quote&body=Hi%20Mocky%2C%0AI%20need%20a%20site.');
    renderApp({}, api);
    const form = within(await screen.findByRole('form', { name: 'New message' }));
    expect(form.getByLabelText('Subject')).toHaveValue('Website quote');
    expect(messageText(await form.findByLabelText('Message'))).toContain('Hi Mocky,\nI need a site.');
    expect(within(form.getByRole('list', { name: 'To recipients' })).getByText(/info@mocky\.co\.ke/)).toBeInTheDocument();
  });

  it('open a new message when a link arrives while the app is open', async () => {
    const { api } = renderApp();
    await rows();
    act(() => {
      api.emit('mail://mailto', { url: 'mailto:peter@client.example?cc=amina@studio.example' });
    });
    const form = within(await screen.findByRole('form', { name: 'New message' }));
    expect(within(form.getByRole('list', { name: 'To recipients' })).getByText(/peter@client\.example/)).toBeInTheDocument();
  });
});
