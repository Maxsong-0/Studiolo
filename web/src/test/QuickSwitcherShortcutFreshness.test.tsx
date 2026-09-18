import { useLayoutEffect } from 'react';
import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { useQuickSwitcher } from '../useQuickSwitcher';

/**
 * 可用状态刚提交、被动效果还没重挂监听的那一瞬按下 ⌘K：
 * React 的提交与被动效果是两个独立调度任务，真实浏览器与 RTL 的
 * findBy*（其 asyncWrapper 会关闭 act 环境）都会落进这段间隙，
 * 快捷键必须按已提交的可用状态判定，不能读上一轮闭包里的旧值。
 */
function ShortcutHarness({ enabled }: { enabled: boolean }) {
  const quick = useQuickSwitcher(enabled);
  // 声明在 useQuickSwitcher 之后，布局阶段必然晚于它刷新可用状态那一步，
  // 又必然早于任何被动效果，正好钉在这段间隙里敲键。
  useLayoutEffect(() => {
    if (!enabled) return;
    window.dispatchEvent(
      new KeyboardEvent('keydown', { key: 'k', metaKey: true }),
    );
  }, [enabled]);
  return <p>{quick.open ? '快速切换已打开' : '快速切换未打开'}</p>;
}

it('可用状态刚提交即按下快捷键也能打开，不等被动效果重挂监听', () => {
  const view = render(<ShortcutHarness enabled={false} />);
  expect(screen.getByText('快速切换未打开')).toBeInTheDocument();
  view.rerender(<ShortcutHarness enabled />);
  expect(screen.getByText('快速切换已打开')).toBeInTheDocument();
});
