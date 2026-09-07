import { act, renderHook } from '@testing-library/react';
import { expect, it } from 'vitest';
import { useDocumentHistory } from '../useDocumentHistory';

it('历史最多保存100个位置，重复位置不堆积', () => {
  const { result } = renderHook(() =>
    useDocumentHistory({ path: 'a.md', line: 1 }),
  );
  act(() => {
    for (let line = 2; line <= 120; line++)
      result.current.navigate({ path: 'a.md', line });
  });
  act(() => result.current.navigate({ path: 'a.md', line: 120 }));
  for (let index = 0; index < 99; index++) act(() => result.current.back());
  expect(result.current.location.line).toBe(21);
  expect(result.current.canGoBack).toBe(false);
});
