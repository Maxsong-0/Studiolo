import { describe, expect, it, vi } from 'vitest';
import {
  SINGLE_KEYS_KEY,
  createSingleKeysPreference,
} from '../../keys/preference';

function memory(initial?: string) {
  const values = new Map<string, string>();
  if (initial !== undefined) values.set(SINGLE_KEYS_KEY, initial);
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

describe('单键快捷键的开关', () => {
  it('缺省是开；关掉只存一个固定的字，再开就把它删掉', () => {
    const storage = memory();
    const preference = createSingleKeysPreference(storage);
    expect(preference.enabled()).toBe(true);
    preference.set(false);
    expect(preference.enabled()).toBe(false);
    expect([...storage.values]).toEqual([[SINGLE_KEYS_KEY, 'off']]);
    preference.set(true);
    expect(preference.enabled()).toBe(true);
    expect(storage.values.size).toBe(0);
  });

  it('上次关掉的，下次打开还是关着；存了别的字当作开', () => {
    expect(createSingleKeysPreference(memory('off')).enabled()).toBe(false);
    expect(createSingleKeysPreference(memory('on')).enabled()).toBe(true);
    expect(createSingleKeysPreference(memory('')).enabled()).toBe(true);
  });

  it('变了才通知；退订之后不再通知', () => {
    const preference = createSingleKeysPreference(memory());
    const listener = vi.fn();
    const unsubscribe = preference.subscribe(listener);
    preference.set(true);
    expect(listener).not.toHaveBeenCalled();
    preference.set(false);
    expect(listener).toHaveBeenCalledTimes(1);
    preference.set(false);
    expect(listener).toHaveBeenCalledTimes(1);
    unsubscribe();
    preference.set(true);
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('别的标签页改了：按本机存储重读一次', () => {
    const storage = memory();
    const preference = createSingleKeysPreference(storage);
    const listener = vi.fn();
    preference.subscribe(listener);
    storage.values.set(SINGLE_KEYS_KEY, 'off');
    preference.sync();
    expect(preference.enabled()).toBe(false);
    expect(listener).toHaveBeenCalledTimes(1);
    preference.sync();
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it('本机存储用不了：读不了当作开，写不了只在这一页里生效，都不报错', () => {
    expect(createSingleKeysPreference(null).enabled()).toBe(true);
    const broken = {
      getItem: () => {
        throw new Error('被禁用');
      },
      setItem: () => {
        throw new Error('配额满');
      },
      removeItem: () => {
        throw new Error('被禁用');
      },
    };
    const preference = createSingleKeysPreference(broken);
    expect(preference.enabled()).toBe(true);
    preference.set(false);
    expect(preference.enabled()).toBe(false);
    preference.set(true);
    expect(preference.enabled()).toBe(true);
    const offline = createSingleKeysPreference(null);
    offline.set(false);
    expect(offline.enabled()).toBe(false);
  });
});
