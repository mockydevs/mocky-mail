import { act, screen, waitFor, within } from '@testing-library/react';

import { reader, renderApp, rows } from '../../test/renderApp';

async function openTasks(options: Parameters<typeof renderApp>[0] = {}) {
  const ctx = renderApp(options);
  await rows();
  await ctx.user.click(screen.getByRole('button', { name: 'Tasks' }));
  const region = within(await screen.findByRole('region', { name: 'Tasks' }));
  await region.findByRole('list', { name: 'All tasks' });
  return { ...ctx, region };
}

const titles = (list: HTMLElement) =>
  within(list)
    .getAllByRole('listitem')
    .map((li) => li.querySelector('.task-row__title')?.textContent);

describe('tasks', () => {
  it('filters tasks: today, the next 7 days, all and completed', async () => {
    const { user, region } = await openTasks();
    expect(titles(region.getByRole('list', { name: 'All tasks' }))).toEqual([
      'Renew the domain',
      'Send the quotation to Acme',
      'Book the venue',
      'Buy printer paper',
    ]);
    const lists = within(region.getByRole('navigation', { name: 'Task lists' }));
    await user.click(lists.getByRole('button', { name: 'Today' }));
    const today = await region.findByRole('list', { name: 'Today tasks' });
    expect(titles(today)).toEqual(['Renew the domain', 'Send the quotation to Acme']);
    const overdue = within(today).getByText(/^Overdue · /);
    expect(overdue).toHaveClass('badge--overdue');
    expect(within(today).getByText('Today')).not.toHaveClass('badge--overdue');
    expect(within(today).getByText('High importance')).toBeInTheDocument();
    expect(within(today).getByText('Clients')).toBeInTheDocument();

    await user.click(lists.getByRole('button', { name: 'Next 7 days' }));
    expect(titles(await region.findByRole('list', { name: 'Next 7 days tasks' }))).toEqual([
      'Renew the domain',
      'Send the quotation to Acme',
      'Book the venue',
    ]);
    await user.click(lists.getByRole('button', { name: 'Completed' }));
    expect(titles(await region.findByRole('list', { name: 'Completed tasks' }))).toEqual(['Pay the October invoice']);
    expect(region.queryByRole('form', { name: 'Add a task' })).not.toBeInTheDocument();
    await user.click(lists.getByRole('button', { name: 'Work' }));
    expect(titles(await region.findByRole('list', { name: 'Work tasks' }))).toEqual(['Renew the domain', 'Send the quotation to Acme']);
  });

  it('adds, completes, edits and deletes tasks', async () => {
    const { user, api, region } = await openTasks();
    await user.type(region.getByRole('textbox', { name: 'Add a task' }), 'Order business cards{Enter}');
    const all = await region.findByRole('list', { name: 'All tasks' });
    await waitFor(() => {
      expect(titles(all)).toContain('Order business cards');
    });
    expect(region.getByRole('textbox', { name: 'Add a task' })).toHaveValue('');
    const added = (await api.listTasks('device:tasks', false)).find((t) => t.title === 'Order business cards');
    expect(added?.due).toBeNull();

    await user.click(region.getByRole('checkbox', { name: 'Complete Buy printer paper' }));
    await waitFor(() => {
      expect(region.queryByRole('checkbox', { name: 'Complete Buy printer paper' })).not.toBeInTheDocument();
    });
    await user.click(region.getByRole('button', { name: 'Completed' }));
    expect(await region.findByRole('checkbox', { name: 'Mark Buy printer paper as not done' })).toBeChecked();

    await user.click(region.getByRole('button', { name: 'All' }));
    await user.click(await region.findByRole('button', { name: /Book the venue/ }));
    const form = within(region.getByRole('form', { name: 'Task' }));
    await user.clear(form.getByRole('textbox', { name: 'Title' }));
    await user.type(form.getByRole('textbox', { name: 'Title' }), 'Book the hall');
    await user.selectOptions(form.getByRole('combobox', { name: 'Importance' }), 'High');
    await user.selectOptions(form.getByRole('combobox', { name: 'List' }), 'Work');
    await user.click(form.getByRole('checkbox', { name: 'Clients' }));
    await user.type(form.getByLabelText('Notes'), 'Seats 40.');
    await user.click(form.getByRole('button', { name: 'Save task' }));
    await waitFor(async () => {
      expect((await api.listTasks('work:tasks', false)).find((t) => t.id === 't-venue')).toMatchObject({
        title: 'Book the hall',
        importance: 'high',
        categories: ['Clients'],
        notes: 'Seats 40.',
      });
    });

    await user.click(await region.findByRole('button', { name: /Book the hall/ }));
    await user.click(within(region.getByRole('form', { name: 'Task' })).getByRole('button', { name: 'Delete' }));
    await user.click(within(screen.getByRole('dialog')).getByRole('button', { name: 'Delete task' }));
    await waitFor(() => {
      expect(region.queryByRole('button', { name: /Book the hall/ })).not.toBeInTheDocument();
    });
  });

  it('syncs, and refreshes when tasks change elsewhere', async () => {
    const { user, api, region } = await openTasks();
    const sync = vi.spyOn(api, 'syncTasks');
    await waitFor(() => {
      expect(region.getByRole('button', { name: 'Sync tasks' })).toBeEnabled();
    });
    await user.click(region.getByRole('button', { name: 'Sync tasks' }));
    expect(sync).toHaveBeenCalled();
    await act(async () => {
      await api.saveTask({
        id: null,
        listId: 'work:tasks',
        title: 'From the server',
        notes: '',
        due: null,
        reminder: null,
        importance: 'normal',
        completed: false,
        messageId: null,
        categories: [],
      });
    });
    expect(await region.findByText('From the server')).toBeInTheDocument();
  });

  it('creates a task from a message, and opens the message from it', async () => {
    const { user, api } = renderApp();
    const list = await rows();
    await user.click(list.getByRole('button', { name: /Revised quotation/ }));
    const pane = within(reader());
    await user.click(await pane.findByRole('button', { name: 'More actions' }));
    await user.click(pane.getByRole('button', { name: 'Create task' }));
    const toast = await screen.findByText('Task added');
    expect(toast).toBeInTheDocument();
    const made = (await api.listTasks(null, false)).find((t) => t.messageId === 'm1');
    expect(made?.title).toBe('Revised quotation for the Westlands office fit-out');

    // The flag menu offers it too.
    await user.click(pane.getByRole('button', { name: 'Flag options' }));
    await user.click(screen.getByRole('menuitem', { name: 'Add task' }));
    await waitFor(async () => {
      expect((await api.listTasks(null, false)).filter((t) => t.messageId === 'm1')).toHaveLength(2);
    });

    const open = screen.getAllByRole('button', { name: 'Open' })[0];
    if (!open) throw new Error('no Open button');
    await user.click(open);
    const form = within(await screen.findByRole('form', { name: 'Task' }));
    await user.click(form.getByRole('button', { name: 'Open message' }));
    await waitFor(() => {
      expect(within(reader()).getByRole('heading', { name: 'Revised quotation for the Westlands office fit-out' })).toBeInTheDocument();
    });
  });

  it('uses a list picker on a phone', async () => {
    vi.stubGlobal(
      'matchMedia',
      vi.fn((query: string) => ({
        matches: query.startsWith('(max-width'),
        media: query,
        addEventListener: vi.fn(),
        removeEventListener: vi.fn(),
      })),
    );
    try {
      const { user } = renderApp();
      await rows();
      await user.click(screen.getByRole('button', { name: 'Folders and accounts' }));
      await user.click(screen.getByRole('button', { name: 'Tasks' }));
      const region = within(await screen.findByRole('region', { name: 'Tasks' }));
      expect(region.queryByRole('navigation', { name: 'Task lists' })).not.toBeInTheDocument();
      await user.selectOptions(region.getByRole('combobox', { name: 'Show' }), 'Today');
      expect(titles(await region.findByRole('list', { name: 'Today tasks' }))).toHaveLength(2);
      await user.click(region.getByRole('button', { name: /Renew the domain/ }));
      expect(region.getByRole('form', { name: 'Task' })).toBeInTheDocument();
      expect(region.queryByRole('list', { name: 'Today tasks' })).not.toBeInTheDocument();
      await user.click(region.getByRole('button', { name: 'Back' }));
      expect(await region.findByRole('list', { name: 'Today tasks' })).toBeInTheDocument();
    } finally {
      vi.unstubAllGlobals();
    }
  });
});
