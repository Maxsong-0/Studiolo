import { act, fireEvent, screen } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { installFileOperationWriter } from '../fileOperationApi';
import { installCanonicalWriter } from '../canonicalApi';
import type { FileOperationRequest } from '../fileOperationModel';
import { canonical } from './canonicalFixtures';
import { renameResponse } from './renameFixtures';
import { deferred } from './sessionFixtures';
import { setup, review } from './renameConfirmationFixtures';

it('确认绑定同一snapshot和目标；IME、改名改回、草稿及保存R2均使原确认失效', async () => {
  const view = await setup();
  const writing = vi.fn(async (request: FileOperationRequest) => {
    if (request.kind !== 'rename') throw new Error('fixture only rename');
    return renameResponse(request);
  });
  const uninstall = installFileOperationWriter(writing);
  const save = installCanonicalWriter(view.disk);
  try {
    await review();
    const checkbox = screen.getByRole('checkbox');
    const target = screen.getByRole('textbox', { name: '目标完整路径' });
    expect(checkbox).toBeChecked();
    act(() => view.controller.composition(true));
    expect(checkbox).not.toBeChecked();
    act(() => view.controller.composition(false));
    fireEvent.click(checkbox);
    fireEvent.change(target, { target: { value: '笔记/另名.md' } });
    fireEvent.change(target, { target: { value: '笔记/新标题.md' } });
    expect(checkbox).not.toBeChecked();
    fireEvent.click(checkbox);
    fireEvent.change(screen.getByRole('textbox', { name: '测试正文' }), {
      target: { value: '保存后的R2' },
    });
    expect(checkbox).not.toBeChecked();
    expect(screen.getByRole('button', { name: '确认重命名' })).toBeDisabled();
    await act(async () => view.controller.save());
    expect(view.controller.getSnapshot().base?.revision).toBe('e'.repeat(64));
    expect(checkbox).not.toBeChecked();
    expect(writing).not.toHaveBeenCalled();
    fireEvent.click(checkbox);
    await act(async () =>
      fireEvent.click(screen.getByRole('button', { name: '确认重命名' })),
    );
    expect(writing).toHaveBeenCalledOnce();
    expect(writing.mock.calls[0][0]).toMatchObject({
      kind: 'rename',
      from: canonical.path,
      to: '笔记/新标题.md',
      expected_revision: 'e'.repeat(64),
    });
    expect(writing.mock.calls[0][0]).not.toHaveProperty('path');
  } finally {
    view.unmount();
    view.controller.stop();
    uninstall();
    save();
  }
});
it('未知只保留两路径及原键，纯文本目标不会变HTML或提供撤回', async () => {
  const view = await setup();
  const pending = deferred<unknown>();
  const writer = vi.fn().mockReturnValue(pending.promise);
  const uninstall = installFileOperationWriter(writer);
  try {
    await review('笔记/<img onerror=1>.md');
    fireEvent.click(screen.getByRole('button', { name: '确认重命名' }));
    expect(view.files.getSnapshot()?.phase).toBe('sending');
    expect(writer).toHaveBeenCalledOnce();
    await act(async () => {
      pending.reject(new TypeError('lost'));
    });
    expect(await screen.findByRole('alert')).toHaveTextContent(
      '目标可能已创建',
    );
    expect(
      screen.getByText('目标：笔记/<img onerror=1>.md'),
    ).toBeInTheDocument();
    expect(view.container.querySelector('img')).toBeNull();
    expect(
      screen.getByRole('button', { name: '核对原请求' }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole('button', { name: '导出请求信息' }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole('button', { name: /撤回|新键/ }),
    ).not.toBeInTheDocument();
  } finally {
    view.unmount();
    view.controller.stop();
    uninstall();
  }
});
it('只有旧enabled能力不显示Rename', async () => {
  const view = await setup(false);
  try {
    await act(async () => {});
    expect(
      screen.queryByRole('button', { name: '重命名文件' }),
    ).not.toBeInTheDocument();
  } finally {
    view.unmount();
    view.controller.stop();
  }
});

it('取消审阅丢弃确认和目标选择，不提交或更改干净正文', async () => {
  const view = await setup();
  const writer = vi.fn();
  const uninstall = installFileOperationWriter(writer);
  try {
    await review();
    fireEvent.click(screen.getByRole('button', { name: '取消重命名审阅' }));
    expect(screen.queryByRole('checkbox')).not.toBeInTheDocument();
    expect(view.controller.getSnapshot()).toMatchObject({
      phase: 'clean',
      draft: canonical.content,
    });
    fireEvent.click(screen.getByRole('button', { name: '重命名文件' }));
    expect(screen.getByRole('textbox', { name: '目标完整路径' })).toHaveValue(
      '',
    );
    expect(screen.getByRole('checkbox')).not.toBeChecked();
    expect(writer).not.toHaveBeenCalled();
  } finally {
    view.unmount();
    view.controller.stop();
    uninstall();
  }
});
