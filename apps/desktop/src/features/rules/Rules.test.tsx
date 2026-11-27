import { fireEvent, screen, within } from '@testing-library/react';
import { vi } from 'vitest';

import type { Rule } from '../../api/types';
import { accountFolders, reader, renderApp, rows } from '../../test/renderApp';

const rule = (patch: Partial<Rule> & Pick<Rule, 'id' | 'name'>): Rule => ({
  enabled: true,
  accountId: 'work',
  match: 'all',
  conditions: [],
  exceptions: [],
  actions: [{ kind: 'flag' }],
  stopProcessing: false,
  keepOnServer: false,
  serverStatus: 'local',
  serverNote: null,
  order: 0,
  ...patch,
});

const SAMPLE: Rule[] = [
  rule({
    id: 'boss',
    name: 'Boss mail',
    conditions: [{ kind: 'from', value: 'wanjiru' }],
    actions: [{ kind: 'flag' }, { kind: 'move', folderId: 'work:Clients' }],
    keepOnServer: true,
    serverStatus: 'server',
  }),
  rule({
    id: 'builds',
    name: 'Builds',
    accountId: null,
    conditions: [{ kind: 'from', value: 'builds@ci.example' }],
    actions: [{ kind: 'markRead' }],
    keepOnServer: true,
    serverStatus: 'unsupported',
    serverNote: 'Rules for all accounts run on this device only.',
  }),
];

async function openRules(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp({ rules: SAMPLE, ...options });
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Settings' }));
  const region = within(screen.getByRole('region', { name: 'Settings' }));
  const list = () => within(region.getByRole('list', { name: 'Rules' }));
  await region.findByRole('heading', { name: 'Rules' });
  await vi.waitFor(() => {
    expect(region.queryByText('Loading rules')).not.toBeInTheDocument();
  });
  return { ...ctx, region, list };
}

const dialog = (name: string | RegExp) => within(screen.getByRole('dialog', { name }));

describe('rules in settings', () => {
  it('lists rules in order with a summary, the account and where they run', async () => {
    const { list } = await openRules();
    const items = list().getAllByRole('listitem');
    expect(items).toHaveLength(2);
    const [boss, builds] = items as [HTMLElement, HTMLElement];
    expect(boss).toHaveTextContent('Boss mail');
    expect(boss).toHaveTextContent('From contains wanjiru → flag, move to Clients');
    expect(boss).toHaveTextContent('Work');
    expect(within(boss).getByText('On server')).toBeInTheDocument();
    expect(builds).toHaveTextContent('All accounts');
    const badge = within(builds).getByText('This device only');
    expect(badge).toHaveAttribute('title', 'Rules for all accounts run on this device only.');
    // Phones can't hover, so the reason is written out too.
    expect(builds).toHaveTextContent('Rules for all accounts run on this device only.');
  });

  it('turns a rule off, reorders with the buttons and by dragging, and deletes after asking', async () => {
    const { user, api, list } = await openRules();
    await user.click(list().getByRole('checkbox', { name: 'Use Boss mail' }));
    await vi.waitFor(async () => {
      expect((await api.listRules())[0]?.enabled).toBe(false);
    });
    expect(list().getByRole('checkbox', { name: 'Use Boss mail' })).not.toBeChecked();

    expect(list().getByRole('button', { name: 'Move Boss mail up' })).toBeDisabled();
    await user.click(list().getByRole('button', { name: 'Move Builds up' }));
    await vi.waitFor(async () => {
      expect((await api.listRules()).map((r) => r.id)).toEqual(['builds', 'boss']);
    });
    expect(list().getAllByRole('listitem')[0]).toHaveTextContent('Builds');

    // Drag Builds back below Boss mail.
    const handle = list().getByRole('button', { name: 'Drag Builds to change the order' });
    const target = list().getByRole('button', { name: 'Drag Boss mail to change the order' }).closest('li');
    if (!target) throw new Error('row expected');
    fireEvent.dragStart(handle);
    fireEvent.dragOver(target);
    fireEvent.drop(target);
    await vi.waitFor(async () => {
      expect((await api.listRules()).map((r) => r.id)).toEqual(['boss', 'builds']);
    });

    await user.click(list().getByRole('button', { name: 'Delete Builds' }));
    const confirm = dialog('Delete Builds?');
    await user.click(confirm.getByRole('button', { name: 'Delete rule' }));
    await vi.waitFor(() => {
      expect(list().getAllByRole('listitem')).toHaveLength(1);
    });
    expect(await api.listRules()).toHaveLength(1);
    expect(await screen.findByText('Rule “Builds” deleted.')).toBeInTheDocument();
  });

  it('runs a rule now on a folder it picks, and says how many it checked and matched', async () => {
    const { user, api, list } = await openRules();
    await user.click(list().getByRole('button', { name: 'Run Boss mail now…' }));
    const picker = dialog('Run “Boss mail” on which folder?');
    // A rule for one account offers that account's folders only.
    expect(picker.queryByRole('heading', { name: 'Personal' })).not.toBeInTheDocument();
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Inbox' }));
    expect(await screen.findByText('Checked 4, matched 1.')).toBeInTheDocument();
    expect(screen.getByText('Ran “Boss mail” on Inbox')).toBeInTheDocument();
    expect(await api.getMessage('m1', false)).toMatchObject({ folderId: 'work:Clients', flagged: true });

    // Every rule at once: folders of every account, under their names.
    await user.click(screen.getByRole('button', { name: 'Run all rules now…' }));
    const all = dialog('Run your rules on which folder?');
    expect(all.getByRole('heading', { name: 'Personal' })).toBeInTheDocument();
    await user.click(all.getByRole('button', { name: 'Work: Inbox' }));
    expect(await screen.findByText('Checked 3, matched 1.')).toBeInTheDocument();
    expect(await api.getMessage('m2', false)).toMatchObject({ unread: false });
  });
});

describe('rule editor', () => {
  it('checks the rule and counts matches as conditions and exceptions change', async () => {
    const { user, region } = await openRules({ rules: [] });
    expect(region.queryByRole('list', { name: 'Rules' })).not.toBeInTheDocument();
    await user.click(region.getByRole('button', { name: 'New rule' }));
    const editor = dialog('New rule');

    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(editor.getByRole('alert')).toHaveTextContent('Give the rule a name.');
    await user.click(editor.getByRole('textbox', { name: 'Name' }));
    await user.paste('Invoices');
    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(editor.getByRole('alert')).toHaveTextContent('Condition 1: type the words or addresses to look for.');

    await user.selectOptions(editor.getByRole('combobox', { name: 'Condition 1' }), 'Subject contains');
    await user.click(editor.getByRole('textbox', { name: 'Condition 1: words or addresses' }));
    await user.paste('invoice');
    expect(await editor.findByText('Matches 1 message in Inbox')).toBeInTheDocument();

    await user.click(editor.getByRole('button', { name: 'Add exception' }));
    await user.selectOptions(editor.getByRole('combobox', { name: 'Exception 1' }), 'Has an attachment');
    expect(editor.queryByRole('textbox', { name: /Exception 1/ })).not.toBeInTheDocument();
    expect(await editor.findByText('Matches 0 messages in Inbox')).toBeInTheDocument();
    await user.click(editor.getByRole('button', { name: 'Remove exception 1' }));
    expect(await editor.findByText('Matches 1 message in Inbox')).toBeInTheDocument();

    // Sizes in KB, importance from a list.
    await user.click(editor.getByRole('button', { name: 'Add condition' }));
    await user.selectOptions(editor.getByRole('combobox', { name: 'Condition 2' }), 'Larger than');
    await user.type(editor.getByRole('spinbutton', { name: 'Condition 2: size in KB' }), '50');
    await user.selectOptions(editor.getByRole('combobox', { name: 'Condition 2' }), 'Importance is');
    expect(editor.getByRole('combobox', { name: 'Condition 2: importance' })).toHaveValue('high');
    expect(await editor.findByText('Matches 0 messages in Inbox')).toBeInTheDocument();
    await user.click(editor.getByRole('radio', { name: 'Any of them' }));
    expect(await editor.findByText('Matches 2 messages in Inbox')).toBeInTheDocument();
    await user.click(editor.getByRole('button', { name: 'Remove condition 2' }));
    expect(await editor.findByText('Matches 1 message in Inbox')).toBeInTheDocument();
  });

  it('picks a folder and a category for the actions, and saves the rule', async () => {
    const { user, api, region, list } = await openRules({ rules: [] });
    await user.click(region.getByRole('button', { name: 'New rule' }));
    const editor = dialog('New rule');
    await user.click(editor.getByRole('textbox', { name: 'Name' }));
    await user.paste('Invoices');
    await user.selectOptions(editor.getByRole('combobox', { name: 'Condition 1' }), 'Subject contains');
    await user.click(editor.getByRole('textbox', { name: 'Condition 1: words or addresses' }));
    await user.paste('invoice');
    await user.click(editor.getByRole('radio', { name: 'Any of them' }));

    // Move: the folder comes from the account's folder tree.
    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(editor.getByRole('alert')).toHaveTextContent('Move to: choose a folder.');
    await user.click(editor.getByRole('button', { name: 'Action 1: folder, none chosen' }));
    const picker = dialog('Move to which folder?');
    expect(picker.getByText('Folders in Work.')).toBeInTheDocument();
    await user.click(within(picker.getByRole('navigation', { name: 'Folders' })).getByRole('button', { name: 'Acme' }));
    expect(editor.getByRole('button', { name: 'Action 1: folder, Acme' })).toBeInTheDocument();

    await user.click(editor.getByRole('button', { name: 'Add action' }));
    await user.selectOptions(editor.getByRole('combobox', { name: 'Action 2' }), 'Add the category');
    await user.selectOptions(editor.getByRole('combobox', { name: 'Action 2: category' }), 'Invoices');
    await user.click(editor.getByRole('checkbox', { name: 'Stop processing more rules' }));
    await user.click(editor.getByRole('checkbox', { name: 'Keep on the mail server' }));
    await user.click(editor.getByRole('button', { name: 'Save rule' }));

    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const [saved] = await api.listRules();
    expect(saved).toMatchObject({
      name: 'Invoices',
      accountId: 'work',
      match: 'any',
      conditions: [{ kind: 'subject', value: 'invoice' }],
      exceptions: [],
      actions: [
        { kind: 'move', folderId: 'work:Clients/Acme' },
        { kind: 'categorize', category: 'Invoices' },
      ],
      stopProcessing: true,
      keepOnServer: true,
    });
    const row = list().getByRole('listitem');
    expect(row).toHaveTextContent('Subject contains invoice → move to Acme, add category Invoices, then stop');
    expect(screen.getByText('Rule “Invoices” added.')).toBeInTheDocument();
    // The server copy is written in the background; mail://rules-changed brings the outcome.
    expect(within(row).getByText('Saving to server…')).toBeInTheDocument();
    expect(row).toHaveTextContent('Being saved to the mail server.');
    expect(await within(row).findByText('This device only')).toBeInTheDocument();
    expect(row).toHaveTextContent('categorise, pin or notify');
    expect((await api.listRules())[0]?.serverStatus).toBe('unsupported');
  });

  it('for all accounts, moves only to folders every account has, and counts the first account’s Inbox', async () => {
    const { user, region } = await openRules({ rules: [] });
    await user.click(region.getByRole('button', { name: 'New rule' }));
    const editor = dialog('New rule');
    await user.selectOptions(editor.getByRole('combobox', { name: 'Account' }), 'All accounts');
    const folder = editor.getByRole('combobox', { name: 'Action 1: folder' });
    expect([...folder.querySelectorAll('option')].map((o) => o.textContent)).toEqual([
      'Choose a folder',
      'Inbox',
      'Archive',
      'Spam',
      'Trash',
    ]);
    expect(editor.getByText(/only move or copy to a folder every account has/)).toBeInTheDocument();
    await user.click(editor.getByRole('textbox', { name: 'Condition 1: words or addresses' }));
    await user.paste('example');
    expect(await editor.findByText(/in Work’s Inbox \(this rule covers all accounts; the first one is counted\)/)).toBeInTheDocument();
    await user.click(editor.getByRole('textbox', { name: 'Name' }));
    await user.paste('Everything');
    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(editor.getByRole('alert')).toHaveTextContent('Move to: choose a folder.');
    await user.selectOptions(folder, 'Archive');
    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(await within(region.getByRole('list', { name: 'Rules' })).findByText(/move to Archive/)).toBeInTheDocument();
  });

  it('asks before throwing away changes, and edits an existing rule', async () => {
    const { user, api, list } = await openRules();
    await user.click(list().getByRole('button', { name: 'Edit Boss mail' }));
    let editor = dialog('Edit rule');
    expect(editor.getByRole('textbox', { name: 'Name' })).toHaveValue('Boss mail');
    expect(editor.getByRole('button', { name: 'Action 2: folder, Clients' })).toBeInTheDocument();
    // Unchanged: Cancel just closes.
    await user.click(editor.getByRole('button', { name: 'Cancel' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();

    await user.click(list().getByRole('button', { name: 'Edit Boss mail' }));
    editor = dialog('Edit rule');
    await user.type(editor.getByRole('textbox', { name: 'Name' }), ' (Wanjiru)');
    await user.keyboard('{Escape}');
    const ask = dialog('Discard changes to this rule?');
    await user.click(ask.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByRole('dialog', { name: 'Edit rule' })).toBeInTheDocument();
    await user.click(dialog('Edit rule').getByRole('button', { name: 'Remove action 1' }));
    await user.click(dialog('Edit rule').getByRole('button', { name: 'Save rule' }));
    await vi.waitFor(async () => {
      expect((await api.listRules())[0]).toMatchObject({ name: 'Boss mail (Wanjiru)', actions: [{ kind: 'move' }] });
    });
    expect(list().getAllByRole('listitem')[0]).toHaveTextContent('Boss mail (Wanjiru)');

    await user.click(list().getByRole('button', { name: 'Edit Builds' }));
    await user.click(dialog('Edit rule').getByRole('checkbox', { name: 'Stop processing more rules' }));
    await user.click(dialog('Edit rule').getByRole('button', { name: 'Cancel' }));
    await user.click(dialog('Discard changes to this rule?').getByRole('button', { name: 'Discard' }));
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    expect((await api.listRules())[1]?.stopProcessing).toBe(false);
  });
});

describe('create rule from mail', () => {
  it('starts from the open message: its sender, and its folder when that is not the Inbox', async () => {
    const { user, api } = renderApp();
    await rows();
    await within(reader()).findByRole('heading', { name: /Westlands/ });
    await user.click(within(reader()).getByRole('button', { name: 'More actions' }));
    await user.click(within(reader()).getByRole('button', { name: 'Create rule…' }));
    let editor = dialog('New rule');
    expect(editor.getByRole('textbox', { name: 'Name' })).toHaveValue('Mail from Wanjiru Kamau');
    expect(editor.getByRole('combobox', { name: 'Account' })).toHaveValue('work');
    expect(editor.getByRole('combobox', { name: 'Condition 1' })).toHaveValue('from');
    expect(editor.getByRole('textbox', { name: 'Condition 1: words or addresses' })).toHaveValue('wanjiru@fitout.example');
    expect(editor.getByRole('button', { name: 'Action 1: folder, none chosen' })).toBeInTheDocument();
    await user.click(editor.getByRole('button', { name: 'Cancel' }));

    await user.click(await accountFolders('Work').findByRole('button', { name: /^Clients/ }));
    await within(reader()).findByRole('heading', { name: 'Logo files, final round' });
    await user.click(within(reader()).getByRole('button', { name: 'More actions' }));
    await user.click(within(reader()).getByRole('button', { name: 'Create rule…' }));
    editor = dialog('New rule');
    expect(editor.getByRole('button', { name: 'Action 1: folder, Clients' })).toBeInTheDocument();
    await user.click(editor.getByRole('button', { name: 'Save rule' }));
    expect(await screen.findByText('Rule saved')).toBeInTheDocument();
    expect((await api.listRules())[0]).toMatchObject({
      conditions: [{ kind: 'from', value: 'amina@studio.example' }],
      actions: [{ kind: 'move', folderId: 'work:Clients' }],
    });
  });

  it('starts from the selected messages: every sender in one condition', async () => {
    const { user } = renderApp();
    await rows();
    await user.click(screen.getByRole('checkbox', { name: /Select Build server/ }));
    await user.click(screen.getByRole('checkbox', { name: /Select Calendar/ }));
    await user.click(screen.getByRole('button', { name: 'Create rule…' }));
    const editor = dialog('New rule');
    expect(editor.getByRole('textbox', { name: 'Name' })).toHaveValue('Mail from these senders');
    expect(editor.getByRole('textbox', { name: 'Condition 1: words or addresses' })).toHaveValue(
      'builds@ci.example, calendar@work.example',
    );
  });
});
