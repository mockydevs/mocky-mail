import { screen, within } from '@testing-library/react';

import { MOCK_ACCOUNTS } from '../../api/mock/mockData';
import { renderApp, rows } from '../../test/renderApp';

async function openSection(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  const section = within(await screen.findByRole('region', { name: 'Automatic replies' }));
  return { ...ctx, section };
}

describe('automatic replies', () => {
  it('switches automatic replies on with dates and a message', async () => {
    const { user, api, section } = await openSection();
    const form = within(await section.findByRole('form', { name: 'Automatic replies' }));
    expect(form.getByRole('status')).toHaveTextContent('Automatic replies are off.');

    await user.click(form.getByRole('checkbox', { name: 'Send automatic replies' }));
    await user.click(form.getByRole('button', { name: 'Save automatic replies' }));
    expect(await section.findByRole('alert')).toHaveTextContent('Write the reply people will get.');

    await user.type(form.getByLabelText('Until (optional)'), '2027-01-04');
    await user.type(form.getByRole('textbox', { name: 'Message' }), "I'm away until 4 January.");
    await user.click(form.getByRole('button', { name: 'Save automatic replies' }));
    expect(await form.findByText(/Saved\. Automatic replies are on until/)).toBeInTheDocument();
    const saved = await api.getOutOfOffice('work');
    expect(saved.active).toBe(true);
    expect(saved.settings.end).toBe('2027-01-04');

    // Another account has its own setting.
    await user.selectOptions(section.getByRole('combobox', { name: 'Account' }), 'Personal (you@personal.example)');
    expect(await section.findByText('Automatic replies are off.')).toBeInTheDocument();
  });

  it('says when the server cannot send automatic replies', async () => {
    const { section } = await openSection({
      outOfOffice: {
        work: {
          accountId: 'work',
          available: false,
          message: "The mail server doesn't offer automatic replies.",
          active: false,
          settings: { enabled: false, subject: 'Out of office', message: '', start: null, end: null, days: 1 },
        },
      },
    });
    expect(await section.findByText("The mail server doesn't offer automatic replies.")).toBeInTheDocument();
    expect(section.queryByRole('form')).not.toBeInTheDocument();
  });

  it('has a separate reply for people outside the organisation on Microsoft accounts', async () => {
    const base = MOCK_ACCOUNTS[0];
    if (!base) throw new Error('sample data missing');
    const outlook = { ...base, id: 'ms', email: 'me@contoso.example', displayName: 'Contoso', auth: 'microsoft' as const, sortOrder: 5 };
    const { user, api, section } = await openSection({ accounts: [...MOCK_ACCOUNTS, outlook] });
    await user.selectOptions(section.getByRole('combobox', { name: 'Account' }), 'Contoso (me@contoso.example)');
    const form = within(await section.findByRole('form', { name: 'Automatic replies' }));
    const audience = form.getByRole('combobox', { name: 'Reply to people outside my organisation' });
    expect(audience).toHaveValue('all');
    await user.click(form.getByRole('checkbox', { name: 'Send automatic replies' }));
    await user.type(form.getByRole('textbox', { name: 'Message' }), 'Away this week.');
    await user.type(form.getByRole('textbox', { name: 'Message for people outside' }), 'Please call the office.');
    await user.selectOptions(audience, 'contacts');
    await user.click(form.getByRole('button', { name: 'Save automatic replies' }));
    expect(await form.findByText(/Saved\./)).toBeInTheDocument();
    expect((await api.getOutOfOffice('ms')).settings).toMatchObject({
      externalAudience: 'contacts',
      externalMessage: 'Please call the office.',
    });
    await user.selectOptions(audience, 'none');
    expect(form.queryByRole('textbox', { name: 'Message for people outside' })).not.toBeInTheDocument();
  });

  it('shows no outside reply for other accounts', async () => {
    const { section } = await openSection();
    const form = within(await section.findByRole('form', { name: 'Automatic replies' }));
    expect(form.queryByRole('combobox', { name: 'Reply to people outside my organisation' })).not.toBeInTheDocument();
  });
});
