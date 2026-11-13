import { useCallback, useEffect, useRef, useState } from 'react';
import { automaticPreview, readAttachment } from './attachmentApi';
import type { Attachment } from './attachmentApi';
import type { TreeEntry } from './treeTypes';

export function useAttachment(entry: TreeEntry, generation: number) {
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<{
    loading: boolean;
    error?: string;
    value?: Attachment;
    url?: string;
  }>({ loading: automaticPreview(entry) });
  const controller = useRef<AbortController | null>(null);
  useEffect(() => {
    if (attempt === 0 && !automaticPreview(entry)) return;
    const current = new AbortController();
    controller.current = current;
    let url: string | undefined;
    setState({ loading: true });
    void readAttachment(entry, generation, current.signal)
      .then((value) => {
        if (current.signal.aborted) return;
        url = URL.createObjectURL(value.blob);
        setState({ loading: false, value, url });
      })
      .catch((error: unknown) => {
        if (!current.signal.aborted)
          setState({
            loading: false,
            error:
              error instanceof Error ? error.message : '附件读取失败，请重试。',
          });
      });
    return () => {
      current.abort();
      if (controller.current === current) controller.current = null;
      if (url) URL.revokeObjectURL(url);
    };
  }, [entry, generation, attempt]);
  return {
    ...state,
    requested: attempt > 0,
    load: () => setAttempt((value) => value + 1),
    cancel: useCallback(() => controller.current?.abort(), []),
  };
}
