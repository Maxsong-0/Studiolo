import { act, fireEvent, render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { SessionBoundary } from '../../SessionBoundary';
import { CanonicalEditor } from '../../components/CanonicalEditor';
import { EditorController } from '../../EditorController';
import { installCanonicalWriter } from '../../canonicalApi';
import type { Replacement } from '../../canonicalModel';
import { useDraftCount } from '../../draftContext';
import { DraftStore } from '../../draftStore';
import { canonical, committed, ownerScope } from '../canonicalFixtures';
import { jsonResponse } from '../fixtures';
import {
  deferred,
  mockSessions,
  pair,
  sessionResponse,
} from '../sessionFixtures';

/*
 * T-161：离开确认框开着时，停顿计时（停止输入 1.5 秒）不许发起自动保存。
 * 否则保存会抢在「放弃」「保留并关闭」之前提交；保存中「放弃草稿并关闭」还会消失、版面移动，点击落空。
 * 这里只验计时这一路，输入表面换成文本框；失焦那一路在 leaveDialog.test.tsx 用真实 CodeMirror 验。
 */
vi.mock('../../components/MarkdownEditor', () => ({
  MarkdownEditor: ({
    value,
    readOnly,
    onChange,
  }: {
    value: string;
    readOnly: boolean;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown 源码"
      value={value}
      readOnly={readOnly}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

function DraftCount() {
  const { count } = useDraftCount();
  return <output aria-label="内存草稿篇数">{count}</output>;
}

async function openEditor() {
  const fetchMock = mockSessions();
  fetchMock.mockImplementation(async (input, init) => {
    if (input === '/api/v1/session')
      return jsonResponse({ ...pair, owner_scope: ownerScope });
    if (String(input).startsWith('/api/v1/file?'))
      return jsonResponse(canonical);
    return sessionResponse(input, init);
  });
  const onClose = vi.fn();
  render(
    <SessionBoundary>
      {() => (
        <>
          <DraftCount />
          <CanonicalEditor path={canonical.path} onClose={onClose} />
        </>
      )}
    </SessionBoundary>,
  );
  const input = await screen.findByRole('textbox', { name: 'Markdown 源码' });
  return {
    input,
    onClose,
    // 保存请求只有这一种：会话链上的 POST /api/v1/mutations。
    saves: () =>
      fetchMock.mock.calls.filter(([path]) => path === '/api/v1/mutations'),
  };
}

/** Esc：浏览器向编辑器对话框发 cancel（jsdom 不会自己发）。 */
function pressEscape() {
  fireEvent(
    screen.getByRole('dialog', { name: '编辑与预览' }),
    new Event('cancel', { cancelable: true }),
  );
}

/** 框里的按钮：编辑器工具行也有一个同名的「导出草稿与恢复信息」，所以限定在框内找。 */
function leaveButton(name: string) {
  const leave = screen.getByRole('alertdialog', { name: '处理未保存草稿' });
  return within(leave).getByRole('button', { name });
}

async function elapse(ms: number) {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(ms);
  });
}

it('确认框开着时停顿计时不发起保存；「继续编辑」后照常自动保存', async () => {
  const { input, saves } = await openEditor();
  vi.useFakeTimers();
  try {
    fireEvent.change(input, { target: { value: '等人决定去留的草稿' } });
    pressEscape();
    expect(leaveButton('继续编辑')).toHaveFocus();
    await elapse(5000);
    expect(saves()).toHaveLength(0);
    expect(screen.getByText('有未保存修改。')).toBeInTheDocument();
    fireEvent.click(leaveButton('继续编辑'));
    await elapse(1499);
    expect(saves()).toHaveLength(0);
    await elapse(1);
    expect(saves()).toHaveLength(1);
  } finally {
    vi.useRealTimers();
  }
});

it('等过计时窗口再点「放弃草稿并关闭」：一次就放弃并关闭，全程不发保存请求', async () => {
  const { input, saves, onClose } = await openEditor();
  vi.useFakeTimers();
  try {
    fireEvent.change(input, { target: { value: '要放弃的草稿' } });
    pressEscape();
    await elapse(5000);
    fireEvent.click(leaveButton('放弃草稿并关闭'));
    expect(onClose).toHaveBeenCalledOnce();
    expect(screen.getByLabelText('内存草稿篇数')).toHaveTextContent('0');
    await elapse(5000);
    expect(saves()).toHaveLength(0);
  } finally {
    vi.useRealTimers();
  }
});

it('等过计时窗口再点「保留内存草稿并关闭」：草稿原样留在内存里，没被自动保存', async () => {
  const { input, saves, onClose } = await openEditor();
  vi.useFakeTimers();
  try {
    fireEvent.change(input, { target: { value: '要留在内存里的草稿' } });
    pressEscape();
    await elapse(5000);
    fireEvent.click(leaveButton('保留内存草稿并关闭'));
    expect(onClose).toHaveBeenCalledOnce();
    await elapse(5000);
    expect(saves()).toHaveLength(0);
    expect(screen.getByLabelText('内存草稿篇数')).toHaveTextContent('1');
  } finally {
    vi.useRealTimers();
  }
});

type Writer = (request: Replacement, signal: AbortSignal) => Promise<unknown>;

/** 控制器单测：真实 EditorController，写入与保存后的核对读取走替身。 */
function controllerHarness() {
  let disk = { ...canonical };
  const writer = vi.fn<Writer>(async (request) => {
    disk = { ...disk, content: request.content, revision: 'b'.repeat(64) };
    return committed(request);
  });
  const uninstall = installCanonicalWriter(writer);
  vi.stubGlobal(
    'fetch',
    vi.fn(async () => jsonResponse(disk)),
  );
  const editor = new EditorController(
    canonical.path,
    new DraftStore().forScope(ownerScope),
  );
  return {
    editor,
    writer,
    /** 在途保存落了盘：之后的核对读到这份正文。 */
    landed(content: string) {
      disk = { ...disk, content, revision: 'b'.repeat(64) };
    },
    stop() {
      editor.stop();
      uninstall();
    },
  };
}

it('控制器：暂停期间停顿计时不保存，解除后仍有未保存修改才重新排定', async () => {
  const harness = controllerHarness();
  const { editor, writer } = harness;
  vi.useFakeTimers();
  try {
    await editor.load();
    editor.holdAutosave(true);
    editor.change('等人决定的草稿');
    await vi.advanceTimersByTimeAsync(5000);
    expect(writer).not.toHaveBeenCalled();
    expect(editor.getSnapshot().phase).toBe('dirty');
    editor.holdAutosave(false);
    await vi.advanceTimersByTimeAsync(1499);
    expect(writer).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1);
    expect(writer).toHaveBeenCalledOnce();
    expect(editor.getSnapshot().phase).toBe('clean');
    editor.holdAutosave(true);
    editor.holdAutosave(false);
    await vi.advanceTimersByTimeAsync(5000);
    expect(writer).toHaveBeenCalledOnce();
  } finally {
    harness.stop();
    vi.useRealTimers();
  }
});

it('控制器：暂停不拦显式保存，也不取消在途请求；回执后留下的改动等解除后再保存', async () => {
  const harness = controllerHarness();
  const { editor, writer } = harness;
  vi.useFakeTimers();
  try {
    await editor.load();
    editor.holdAutosave(true);
    editor.change('显式保存');
    await editor.save();
    expect(writer).toHaveBeenCalledOnce();
    expect(editor.getSnapshot().phase).toBe('clean');

    const response = deferred<unknown>();
    writer.mockImplementationOnce(() => response.promise);
    editor.change('已发送');
    const saving = editor.save();
    editor.change('已发送，继续输入');
    harness.landed('已发送');
    response.resolve(committed(writer.mock.calls[1][0]));
    await saving;
    expect(editor.getSnapshot()).toMatchObject({
      phase: 'dirty',
      draft: '已发送，继续输入',
    });
    await vi.advanceTimersByTimeAsync(5000);
    expect(writer).toHaveBeenCalledTimes(2);
    editor.holdAutosave(false);
    await vi.advanceTimersByTimeAsync(1500);
    expect(writer).toHaveBeenCalledTimes(3);
    expect(writer.mock.calls[2][0]).toMatchObject({
      content: '已发送，继续输入',
    });
  } finally {
    harness.stop();
    vi.useRealTimers();
  }
});
