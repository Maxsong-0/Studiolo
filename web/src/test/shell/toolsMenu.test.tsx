import { useContext, useRef, useState } from 'react';
import { act, fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../../components/AppShell';
import { ToolsMenuContext } from '../../shell/toolsMenuContext';
import { useDialogSurface } from '../../useDialogSurface';

function Dialog({ close }: { close: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(ref, close);
  return (
    <dialog ref={ref} aria-label="账户与会话">
      <button onClick={dismiss}>关闭账户</button>
    </dialog>
  );
}

function Guide() {
  const tools = useContext(ToolsMenuContext);
  return (
    <button onClick={() => tools?.reveal('[data-account-entry]')}>
      前往账户
    </button>
  );
}

function Harness({ note }: { note?: string }) {
  const [dialog, setDialog] = useState(false);
  return (
    <AppShell
      view="cockpit"
      onNavigate={vi.fn()}
      actionCount={12}
      documentCount={7}
      sourceLabel="测试库"
      status={<Guide />}
      toolsNote={note}
      tools={
        <>
          <button data-account-entry onClick={() => setDialog(true)}>
            账户
          </button>
          {dialog ? <Dialog close={() => setDialog(false)} /> : null}
        </>
      }
    >
      <button>外部按钮</button>
      <button onClick={() => setDialog(true)}>命令面板打开账户</button>
    </AppShell>
  );
}

const more = () => screen.getByRole('button', { name: /^更多/ });

it('展开、Esc、点外面和焦点移出都收起；Esc 将焦点还给更多', async () => {
  const user = userEvent.setup();
  render(<Harness />);
  expect(more()).toHaveAttribute('aria-expanded', 'false');
  await user.click(more());
  expect(more()).toHaveAttribute('aria-expanded', 'true');
  screen.getByRole('button', { name: '账户' }).focus();
  await user.keyboard('{Escape}');
  expect(more()).toHaveAttribute('aria-expanded', 'false');
  expect(more()).toHaveFocus();
  await user.click(more());
  fireEvent.pointerDown(document.body);
  expect(more()).toHaveAttribute('aria-expanded', 'false');
  await user.click(more());
  act(() => {
    screen.getByRole('button', { name: '外部按钮' }).focus();
  });
  expect(more()).toHaveAttribute('aria-expanded', 'false');
});

it('入口打开对话框后收菜单，关闭对话框将焦点还给更多', async () => {
  const user = userEvent.setup();
  render(<Harness />);
  await user.click(more());
  await user.click(screen.getByRole('button', { name: '账户' }));
  expect(more()).toHaveAttribute('aria-expanded', 'false');
  expect(
    screen.getByRole('dialog', { name: '账户与会话' }),
  ).toBeInTheDocument();
  await user.click(screen.getByRole('button', { name: '关闭账户' }));
  expect(screen.queryByRole('dialog')).toBeNull();
  expect(more()).toHaveFocus();
});

it('收着的入口仍可由命令打开，对话框的 Esc 不冒充菜单的 Esc', async () => {
  const user = userEvent.setup();
  render(<Harness />);
  await user.click(screen.getByRole('button', { name: '命令面板打开账户' }));
  expect(screen.getByRole('dialog')).toBeInTheDocument();
  fireEvent.click(more());
  fireEvent.keyDown(screen.getByRole('button', { name: '关闭账户' }), {
    key: 'Escape',
  });
  expect(more()).toHaveAttribute('aria-expanded', 'true');
});

it('草稿和文件操作标记写进读屏名字，带路先展开再聚焦入口，切外观不收菜单', async () => {
  const user = userEvent.setup();
  render(<Harness note="文件操作待核对；草稿 2" />);
  expect(more()).toHaveAccessibleName('更多（文件操作待核对；草稿 2）');
  await user.click(screen.getByRole('button', { name: '前往账户' }));
  expect(more()).toHaveAttribute('aria-expanded', 'true');
  expect(screen.getByRole('button', { name: '账户' })).toHaveFocus();
  await user.click(screen.getByRole('button', { name: '深色' }));
  expect(more()).toHaveAttribute('aria-expanded', 'true');
  expect(screen.getByRole('button', { name: '深色' })).toHaveAttribute(
    'aria-pressed',
    'true',
  );
  await user.click(screen.getByRole('button', { name: '跟随系统' }));
});
