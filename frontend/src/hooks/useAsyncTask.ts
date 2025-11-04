import { useCallback, useRef, useState } from 'react';

export type AsyncStatus = 'idle' | 'pending' | 'success' | 'error';

export interface AsyncTaskState<E = Error> {
  status: AsyncStatus;
  error?: E;
}

export function useAsyncTask<A extends unknown[], R, E = Error>(task: (...args: A) => Promise<R>) {
  const [state, setState] = useState<AsyncTaskState<E>>({ status: 'idle' });
  const lastPromise = useRef<Promise<R> | null>(null);

  const run = useCallback(
    async (...args: A): Promise<R> => {
      const promise = task(...args);
      lastPromise.current = promise;
      setState({ status: 'pending' });
      try {
        const result = await promise;
        if (lastPromise.current === promise) {
          setState({ status: 'success' });
        }
        return result;
      } catch (err) {
        if (lastPromise.current === promise) {
          setState({ status: 'error', error: err as E });
        }
        throw err;
      }
    },
    [task],
  );

  const reset = useCallback(() => setState({ status: 'idle' }), []);

  return {
    run,
    reset,
    status: state.status,
    error: state.error,
  };
}
