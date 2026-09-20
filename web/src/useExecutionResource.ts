import { useEffect, useState } from 'react';
import { executionFailure } from './executionApi';

export type Resource<T> = {
  data: T | null;
  checkedAt: number;
  error: string;
  loading: boolean;
};
export function isFresh<T>(resource: Resource<T>): boolean {
  return resource.data !== null && Date.now() - resource.checkedAt < 30_000;
}
/** Poll failures clear old authority; aborted or late results cannot restore it. */
export function useExecutionResource<T>(
  load: (signal: AbortSignal) => Promise<T>,
  revision: number,
  poll = true,
): Resource<T> {
  const [resource, setResource] = useState<Resource<T>>({
    data: null,
    checkedAt: 0,
    error: '',
    loading: true,
  });
  useEffect(() => {
    let active = true;
    let controller: AbortController | null = null;
    let timeout: number | undefined;
    let expires: number | undefined;
    setResource({ data: null, checkedAt: 0, error: '', loading: true });
    const refresh = async () => {
      controller?.abort();
      window.clearTimeout(timeout);
      const current = new AbortController();
      controller = current;
      timeout = window.setTimeout(() => {
        current.abort();
        if (active && controller === current)
          setResource({
            data: null,
            checkedAt: 0,
            error: '连接超时，当前状态不可确认。',
            loading: false,
          });
      }, 10_000);
      try {
        const data = await load(current.signal);
        if (!active || current.signal.aborted || controller !== current) return;
        window.clearTimeout(expires);
        setResource({ data, checkedAt: Date.now(), error: '', loading: false });
        if (poll)
          expires = window.setTimeout(() => {
            if (active)
              setResource({
                data: null,
                checkedAt: 0,
                error: '状态已过期，请重新核对。',
                loading: false,
              });
          }, 30_000);
      } catch (error) {
        if (active && !current.signal.aborted && controller === current)
          setResource({
            data: null,
            checkedAt: 0,
            error: executionFailure(error),
            loading: false,
          });
      } finally {
        if (controller === current) window.clearTimeout(timeout);
      }
    };
    void refresh();
    const interval = poll
      ? window.setInterval(() => {
          void refresh();
        }, 15_000)
      : undefined;
    return () => {
      active = false;
      controller?.abort();
      window.clearTimeout(timeout);
      window.clearTimeout(expires);
      window.clearInterval(interval);
    };
  }, [load, revision, poll]);
  return resource;
}
