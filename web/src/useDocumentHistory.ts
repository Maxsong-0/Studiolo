import { useEffect, useState } from 'react';
import type { SourceLocation } from './types';

export function useDocumentHistory(source: SourceLocation) {
  const [history, setHistory] = useState<SourceLocation[]>([source]);
  const { path, line } = source;
  useEffect(() => setHistory([{ path, line }]), [path, line]);
  return {
    location: history[history.length - 1],
    canGoBack: history.length > 1,
    navigate: (next: SourceLocation) =>
      setHistory((current) => {
        const previous = current[current.length - 1];
        return previous.path === next.path && previous.line === next.line
          ? current
          : [...current.slice(-99), next];
      }),
    back: () =>
      setHistory((current) =>
        current.length > 1 ? current.slice(0, -1) : current,
      ),
  };
}
