import { fireEvent, render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { KeyboardSheet } from '../../keys/KeyboardSheet';
import {
  SINGLE_KEYS_KEY,
  createSingleKeysPreference,
} from '../../keys/preference';
import { GO_TARGETS, SHORTCUT_GROUPS } from '../../keys/shortcuts';
import { KEYBOARD_SHEET_ATTRIBUTE } from '../../keys/useGlobalKeys';

function memory() {
  const values = new Map<string, string>();
  return {
    values,
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => {
      values.set(key, value);
    },
    removeItem: (key: string) => {
      values.delete(key);
    },
  };
}

function renderSheet() {
  const storage = memory();
  const preference = createSingleKeysPreference(storage);
  const onClose = vi.fn();
  render(<KeyboardSheet preference={preference} onClose={onClose} />);
  const dialog = screen.getByRole('dialog', { name: '键盘快捷键' });
  return { storage, preference, onClose, dialog };
}

describe('快捷键一览', () => {
  it('是一个带标记的原生弹层，按表里的三组列出全部键位', () => {
    const { dialog } = renderSheet();
    expect(dialog).toHaveAttribute('open');
    expect(dialog).toHaveAttribute(KEYBOARD_SHEET_ATTRIBUTE);
    const groups = within(dialog).getAllByRole('region');
    expect(
      groups.map(
        (group) => within(group).getByRole('heading', { level: 3 }).textContent,
      ),
    ).toEqual(SHORTCUT_GROUPS.map((group) => `${group.title}${group.where}`));
    for (const [index, group] of SHORTCUT_GROUPS.entries())
      expect(
        within(groups[index])
          .getAllByRole('definition')
          .map((cell) => cell.textContent),
      ).toEqual(group.rows.map((row) => row.does));
    // 跳转：每个字母一行，写成「G 再按 字母」。
    const jump = groups[1];
    for (const { key, label } of GO_TARGETS) {
      const row = within(jump).getByText(`去${label}`).closest('div')!;
      expect(row.querySelector('dt')).toHaveTextContent(
        `G再按${key.toUpperCase()}`,
      );
    }
    // 组合键的两种按法之间写「或」。
    expect(
      within(groups[0])
        .getByText(/打开或收起动作面板/)
        .closest('div')!
        .querySelector('dt'),
    ).toHaveTextContent('⌘K或CtrlK');
  });

  it('开关缺省是开；关掉后单键的两组标「已关闭」，⌘K 那一组不变，并且记进本机存储', async () => {
    const user = userEvent.setup();
    const { dialog, storage, preference } = renderSheet();
    const toggle = within(dialog).getByRole('checkbox', {
      name: '启用单键快捷键',
    });
    expect(toggle).toBeChecked();
    expect(toggle).toHaveAccessibleDescription(/只记在这台浏览器里/);
    expect(dialog.querySelectorAll('[data-off]')).toHaveLength(0);

    await user.click(toggle);
    expect(toggle).not.toBeChecked();
    expect(preference.enabled()).toBe(false);
    expect(storage.values.get(SINGLE_KEYS_KEY)).toBe('off');
    const groups = within(dialog).getAllByRole('region');
    expect(groups.map((group) => group.hasAttribute('data-off'))).toEqual(
      SHORTCUT_GROUPS.map((group) => group.single),
    );
    expect(within(groups[1]).getByRole('heading')).toHaveTextContent('已关闭');
    expect(within(groups[0]).getByRole('heading')).not.toHaveTextContent(
      '已关闭',
    );

    await user.click(toggle);
    expect(toggle).toBeChecked();
    expect(storage.values.has(SINGLE_KEYS_KEY)).toBe(false);
    expect(dialog.querySelectorAll('[data-off]')).toHaveLength(0);
  });

  it('关闭按钮收起，并且只回调一次', async () => {
    const user = userEvent.setup();
    const first = renderSheet();
    await user.click(
      within(first.dialog).getByRole('button', { name: '关闭键盘快捷键' }),
    );
    expect(first.onClose).toHaveBeenCalledTimes(1);
    expect(first.dialog).not.toHaveAttribute('open');
  });

  it('Esc 走 dialog 的 cancel：拦下浏览器的直接关闭，自己收起', () => {
    const { dialog, onClose } = renderSheet();
    const cancel = new Event('cancel', { cancelable: true });
    fireEvent(dialog, cancel);
    expect(cancel.defaultPrevented).toBe(true);
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(dialog).not.toHaveAttribute('open');
  });
});
