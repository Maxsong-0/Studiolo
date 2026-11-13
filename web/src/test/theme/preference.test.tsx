import { act, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ThemeSwitch } from '../../theme/ThemeSwitch';
import {
  applyTheme,
  createThemePreference,
  startTheme,
  themePreference,
  THEME_KEY,
} from '../../theme/preference';
import { themeBootPath, themeBootSource } from '../../../dev/themeBootPlugin';

afterEach(() => {
  localStorage.clear();
  document.documentElement.removeAttribute('data-theme');
});

describe('深浅色偏好', () => {
  it('只存两个固定字，跟随系统删键；通知可退订', () => {
    const preference = createThemePreference(localStorage);
    const changed = vi.fn();
    const stop = preference.subscribe(changed);
    expect(preference.choice()).toBe('system');
    preference.set('dark');
    expect(localStorage.getItem(THEME_KEY)).toBe('dark');
    preference.set('dark');
    expect(changed).toHaveBeenCalledTimes(1);
    preference.set('light');
    expect(localStorage.getItem(THEME_KEY)).toBe('light');
    preference.set('system');
    expect(localStorage.getItem(THEME_KEY)).toBeNull();
    stop();
    preference.set('dark');
    expect(changed).toHaveBeenCalledTimes(3);
  });

  it('未知存值和读失败回到系统；写失败仍在当前页切换', () => {
    localStorage.setItem(THEME_KEY, '未知');
    expect(createThemePreference(localStorage).choice()).toBe('system');
    const denied = () => {
      throw new Error('存储被禁用');
    };
    const preference = createThemePreference({
      getItem: denied,
      setItem: denied,
      removeItem: denied,
    });
    expect(preference.choice()).toBe('system');
    preference.set('dark');
    expect(preference.choice()).toBe('dark');
    preference.set('system');
    expect(preference.choice()).toBe('system');
    expect(createThemePreference(null).choice()).toBe('system');
  });

  it('启动、点击和别的标签页修改都同步根属性；停止后不再写根元素', () => {
    const preference = createThemePreference(localStorage);
    const stop = startTheme(document, preference);
    render(<ThemeSwitch preference={preference} />);
    expect(screen.getByRole('button', { name: '跟随系统' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    fireEvent.click(screen.getByRole('button', { name: '深色' }));
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    localStorage.setItem(THEME_KEY, 'light');
    act(() => preference.sync());
    expect(screen.getByRole('button', { name: '浅色' })).toHaveAttribute(
      'aria-pressed',
      'true',
    );
    expect(document.documentElement).toHaveAttribute('data-theme', 'light');
    fireEvent.click(screen.getByRole('button', { name: '跟随系统' }));
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    stop();
    act(() => preference.set('dark'));
    expect(document.documentElement).not.toHaveAttribute('data-theme');
  });

  it('storage 只响应本机存储的主题键和清空事件', () => {
    const preference = themePreference();
    preference.sync();
    const stop = startTheme(document, preference);
    localStorage.setItem(THEME_KEY, 'dark');
    window.dispatchEvent(
      new StorageEvent('storage', { key: 'other', storageArea: localStorage }),
    );
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    window.dispatchEvent(
      new StorageEvent('storage', {
        key: THEME_KEY,
        storageArea: sessionStorage,
      }),
    );
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    window.dispatchEvent(
      new StorageEvent('storage', {
        key: THEME_KEY,
        storageArea: localStorage,
      }),
    );
    expect(document.documentElement).toHaveAttribute('data-theme', 'dark');
    localStorage.clear();
    window.dispatchEvent(
      new StorageEvent('storage', { key: null, storageArea: localStorage }),
    );
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    stop();
  });

  it('重复套用不改无关根属性', () => {
    const root = document.createElement('html');
    root.lang = 'zh';
    applyTheme(root, 'light');
    applyTheme(root, 'light');
    applyTheme(root, 'system');
    expect(root.lang).toBe('zh');
    expect(root).not.toHaveAttribute('data-theme');
  });
});

describe('首屏外部脚本', () => {
  it.each(['light', 'dark', '未知', null])(
    '原文执行读取 %s，与应用同一个键和取值',
    (value) => {
      if (value !== null) localStorage.setItem(THEME_KEY, value);
      // 测的是发布产物实际执行的经典脚本，不能用应用函数冒充。
      window.eval(themeBootSource());
      expect(document.documentElement.getAttribute('data-theme')).toBe(
        value === 'light' || value === 'dark' ? value : null,
      );
    },
  );

  it('存储被禁用不阻止页面启动，脚本名带八位内容哈希', () => {
    vi.spyOn(Storage.prototype, 'getItem').mockImplementation(() => {
      throw new Error('不可用');
    });
    expect(() => window.eval(themeBootSource())).not.toThrow();
    expect(document.documentElement).not.toHaveAttribute('data-theme');
    expect(themeBootPath()).toMatch(
      /^assets\/theme-boot-[A-Za-z0-9_-]{8}\.js$/,
    );
    expect(themeBootPath(themeBootSource() + ' ')).not.toBe(themeBootPath());
  });
});
