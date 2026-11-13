import { act, renderHook } from '@testing-library/react';

import { useLoad } from './useLoad';

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

describe('useLoad', () => {
  it('ignores a slow result for a resource the user already left', async () => {
    const slow = deferred<string>();
    const fast = deferred<string>();
    const { result, rerender } = renderHook(
      ({ resource, p }: { resource: string; p: Promise<string> }) => useLoad(resource, 0, () => p),
      { initialProps: { resource: 'inbox', p: slow.promise } },
    );
    expect(result.current).toEqual({ status: 'loading' });

    rerender({ resource: 'sent', p: fast.promise });
    await act(async () => {
      fast.resolve('sent list');
      slow.resolve('inbox list');
      await Promise.resolve();
    });
    expect(result.current).toEqual({ status: 'ready', value: 'sent list' });
  });

  it('keeps the current value while a new revision loads', async () => {
    const next = deferred<number>();
    const { result, rerender } = renderHook(
      ({ revision, p }: { revision: number; p: Promise<number> }) => useLoad('inbox', revision, () => p),
      { initialProps: { revision: 0, p: Promise.resolve(1) } },
    );
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current).toEqual({ status: 'ready', value: 1 });

    rerender({ revision: 1, p: next.promise });
    expect(result.current).toEqual({ status: 'ready', value: 1 });
    await act(async () => {
      next.resolve(2);
      await Promise.resolve();
    });
    expect(result.current).toEqual({ status: 'ready', value: 2 });
  });

  it('reports errors', async () => {
    const { result } = renderHook(() => useLoad('x', 0, () => Promise.reject(new Error('disk full'))));
    await act(async () => {
      await Promise.resolve();
    });
    expect(result.current).toEqual({ status: 'error', message: 'disk full' });
  });
});
