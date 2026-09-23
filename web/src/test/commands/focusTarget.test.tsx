import { act, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import {
  COMMAND_TARGETS,
  focusCommandTarget,
} from '../../commands/focusTarget';

beforeEach(() => {
  vi.useFakeTimers();
  // 固定走定时器分支，帧间隔 16ms，便于按时间推进。
  vi.stubGlobal('requestAnimationFrame', undefined);
});
afterEach(() => {
  vi.useRealTimers();
});

function LateSurface({ show }: { show: boolean }) {
  return show ? (
    <button data-command-target={COMMAND_TARGETS.policyQueue}>待处理</button>
  ) : (
    <p>读取中</p>
  );
}

function Harness() {
  const [show, setShow] = useState(false);
  return (
    <>
      <button onClick={() => setShow(true)}>入口</button>
      <LateSurface show={show} />
    </>
  );
}

it('目标晚于跳转才渲染出来也能接住焦点', () => {
  render(<Harness />);
  const entry = screen.getByRole('button', { name: '入口' });
  entry.focus();
  focusCommandTarget(COMMAND_TARGETS.policyQueue);
  act(() => {
    vi.advanceTimersByTime(100);
  });
  expect(entry).toHaveFocus();
  act(() => {
    entry.click();
  });
  act(() => {
    vi.advanceTimersByTime(32);
  });
  expect(screen.getByRole('button', { name: '待处理' })).toHaveFocus();
});

it('人自己挪了焦点、另有对话框开着或等过了时限，都不抢焦点', () => {
  const { rerender } = render(<LateSurface show={false} />);
  const other = document.createElement('input');
  document.body.append(other);
  try {
    focusCommandTarget(COMMAND_TARGETS.policyQueue);
    act(() => {
      vi.advanceTimersByTime(16);
    });
    other.focus();
    rerender(<LateSurface show />);
    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(other).toHaveFocus();

    const dialog = document.createElement('dialog');
    dialog.setAttribute('open', '');
    document.body.append(dialog);
    focusCommandTarget(COMMAND_TARGETS.policyQueue);
    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(other).toHaveFocus();
    dialog.remove();

    rerender(<LateSurface show={false} />);
    focusCommandTarget(COMMAND_TARGETS.policyQueue);
    act(() => {
      vi.advanceTimersByTime(2100);
    });
    rerender(<LateSurface show />);
    act(() => {
      vi.advanceTimersByTime(100);
    });
    expect(other).toHaveFocus();
  } finally {
    other.remove();
  }
});

it('取消后不再尝试', () => {
  const { rerender } = render(<LateSurface show={false} />);
  const cancel = focusCommandTarget(COMMAND_TARGETS.policyQueue);
  cancel();
  rerender(<LateSurface show />);
  act(() => {
    vi.advanceTimersByTime(100);
  });
  expect(screen.getByRole('button', { name: '待处理' })).not.toHaveFocus();
});
