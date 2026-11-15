import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, beforeEach, vi } from 'vitest';

import { MailApiProvider } from '../../api/MailApiContext';
import { createMockMailApi } from '../../api/mock/mockMailApi';
import type { UpdateInfo } from '../../api/types';
import { CHECK_EVERY_MS, FIRST_CHECK_MS, UpdateBanner } from './UpdateBanner';

const INSTALL: UpdateInfo = { version: '0.3.0', notes: 'Faster sync.', how: 'install', page: 'https://mailer.co.ke/' };

function show(update: UpdateInfo | undefined) {
  const api = createMockMailApi(update ? { update } : {});
  render(
    <MailApiProvider api={api}>
      <UpdateBanner />
    </MailApiProvider>,
  );
  return api;
}

async function afterFirstCheck() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(FIRST_CHECK_MS + 10);
  });
}

describe('update banner', () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
    window.localStorage.clear();
  });

  it('stays hidden when there is nothing newer', async () => {
    show(undefined);
    await afterFirstCheck();
    expect(screen.queryByRole('status')).not.toBeInTheDocument();
  });

  it('offers a newer version after start-up and installs it on request', async () => {
    const api = show(INSTALL);
    const install = vi.spyOn(api, 'installUpdate');
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
    await afterFirstCheck();
    expect(screen.getByText('Mocky Mail 0.3.0 is available')).toBeInTheDocument();
    expect(screen.getByText('Faster sync.')).toBeInTheDocument();
    fireEvent.click(screen.getByRole('button', { name: 'Update now' }));
    expect(install).toHaveBeenCalledTimes(1);
    expect(screen.getByText('Downloading and checking the update…')).toBeInTheDocument();
  });

  it('shows why an update failed', async () => {
    const api = show(INSTALL);
    await afterFirstCheck();
    api.failNext('installUpdate', { kind: 'invalid', message: "The downloaded update didn't match the published checksum." });
    await act(async () => {
      fireEvent.click(screen.getByRole('button', { name: 'Update now' }));
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(screen.getByRole('alert')).toHaveTextContent('checksum');
    expect(screen.getByRole('button', { name: 'Update now' })).toBeEnabled();
  });

  it('opens the download page for copies that cannot update themselves', async () => {
    const api = show({ ...INSTALL, how: 'download' });
    await afterFirstCheck();
    fireEvent.click(screen.getByRole('button', { name: 'Download' }));
    await act(async () => {
      await vi.advanceTimersByTimeAsync(10);
    });
    expect(api.opened).toEqual(['https://mailer.co.ke/']);
  });

  it('"Later" hides this version, and checks again every few hours', async () => {
    const api = show(INSTALL);
    const check = vi.spyOn(api, 'checkForUpdate');
    await afterFirstCheck();
    fireEvent.click(screen.getByRole('button', { name: 'Later' }));
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
    await act(async () => {
      await vi.advanceTimersByTimeAsync(CHECK_EVERY_MS);
    });
    expect(check).toHaveBeenCalledTimes(2);
    // Same version: still hidden.
    expect(screen.queryByText(/is available/)).not.toBeInTheDocument();
  });
});
