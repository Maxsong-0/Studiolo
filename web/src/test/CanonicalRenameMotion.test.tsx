import { act, fireEvent, screen, waitFor } from '@testing-library/react';
import { expect, it } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { renaming } from './renameFixtures';
import { jsonResponse } from './fixtures';
import { deferred } from './sessionFixtures';
import { mediaPreference } from './dialogSurfaceHelpers';
import { setup, service, start, fresh, accept } from './renameBrowserFixtures';

// 规范第 11 节：浮层出现只做 120ms 透明度＋4px 位移，减少动效时取消位移只留透明度。
// 动效由共用的 useDialogSurface 驱动；这里只断言与曲线无关的事实：入场从透明淡入到不透明，
// 减少动效时任何时刻都没有位移或缩放。本机负载高，等待上限放宽，不据此断言时长。
const noDisplacement = ['', 'none'];

it('取消关闭动画不复活旧读取，但仍允许新的手动目标核验', async () => {
  mediaPreference(false);
  const waiting = deferred<Response>();
  let reads = 0;
  service(async () => (++reads === 1 ? waiting.promise : jsonResponse(fresh)));
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    // 与 start() 同一串步骤，只是在打开的那一刻先看一眼入场的起点。
    fireEvent.click(screen.getByRole('button', { name: '打开源' }));
    const dialog = screen.getByRole('dialog', { name: '编辑与预览' });
    expect(Number(dialog.style.opacity)).toBeLessThan(1);
    await screen.findByRole('textbox', { name: 'Markdown 源码' });
    const rename = await screen.findByRole('button', { name: '重命名文件' });
    await waitFor(() => expect(rename).toBeEnabled());
    fireEvent.click(rename);
    fireEvent.change(screen.getByRole('textbox', { name: '目标完整路径' }), {
      target: { value: renaming.to },
    });
    fireEvent.click(screen.getByRole('checkbox'));
    fireEvent.click(screen.getByRole('button', { name: '确认重命名' }));
    await screen.findByRole('button', { name: '正在核验目标…' });
    await waitFor(() => expect(dialog.style.opacity).toBe('1'), {
      timeout: 1500,
    });
    fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
    fireEvent.click(screen.getByRole('button', { name: '隐藏预览' }));
    expect(dialog).toHaveAttribute('open');
    await act(async () => {
      waiting.resolve(jsonResponse(fresh));
    });
    const retry = await screen.findByRole('button', { name: '重新读取目标' });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
      renaming.from,
    );
    expect(reads).toBe(1);
    fireEvent.click(retry);
    await waitFor(() =>
      expect(screen.getByLabelText('已打开路径')).toHaveTextContent(
        renaming.to,
      ),
    );
    expect(reads).toBe(2);
  } finally {
    view.unmount();
    uninstall();
  }
});

it('减少动效时浮层不做位移；关闭后迟到的核验结果仍不打开目标', async () => {
  mediaPreference(true);
  const waiting = deferred<Response>();
  service(() => waiting.promise);
  const uninstall = installFileOperationWriter(accept);
  const view = setup();
  try {
    await start();
    await screen.findByRole('button', { name: '正在核验目标…' });
    const dialog = screen.getByRole('dialog', { name: '编辑与预览' });
    expect(noDisplacement).toContain(dialog.style.transform);
    await waitFor(() => expect(dialog.style.opacity).toBe('1'), {
      timeout: 1500,
    });
    expect(noDisplacement).toContain(dialog.style.transform);
    fireEvent.click(screen.getByRole('button', { name: '关闭编辑器' }));
    expect(noDisplacement).toContain(dialog.style.transform);
    await waitFor(
      () =>
        expect(screen.getByLabelText('已打开路径')).toHaveTextContent('closed'),
      { timeout: 1500 },
    );
    await act(async () => {
      waiting.resolve(jsonResponse(fresh));
    });
    expect(screen.getByLabelText('已打开路径')).toHaveTextContent('closed');
    expect(view.files.getSnapshot()?.phase).toBe('committed');
  } finally {
    view.unmount();
    uninstall();
  }
});
