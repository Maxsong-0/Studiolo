import { describe, expect, it } from 'vitest';
import {
  defaultViewLocation,
  formatViewLocation,
  parseViewLocation,
  sameViewLocation,
} from '../viewLocation';

describe('视图位置', () => {
  it('解析视图与三格导航状态，`#` 可有可无', () => {
    const expected = {
      view: 'board' as const,
      project: '项目/星舟',
      panel: 'replay',
      item: 'T-001',
    };
    expect(
      parseViewLocation(
        '#view=board&project=%E9%A1%B9%E7%9B%AE%2F%E6%98%9F%E8%88%9F&panel=replay&item=T-001',
      ),
    ).toEqual(expected);
    expect(
      parseViewLocation(
        'view=board&project=%E9%A1%B9%E7%9B%AE%2F%E6%98%9F%E8%88%9F&panel=replay&item=T-001',
      ),
    ).toEqual(expected);
  });

  it('未知视图、空 hash 与损坏内容一律回落总览，不抛错也不保留子键', () => {
    for (const hash of [
      '',
      '#',
      '#view=',
      '#view=unknown&project=%E9%A1%B9%E7%9B%AE%2F%E6%98%9F%E8%88%9F',
      '#%%%',
      '#project=%E9%A1%B9%E7%9B%AE%2F%E6%98%9F%E8%88%9F',
    ])
      expect(parseViewLocation(hash)).toEqual(defaultViewLocation);
  });

  it('空值、超长值与控制字符不算位置，不把 URL 里的外部输入原样回显', () => {
    const long = 'a'.repeat(513);
    const parsed = parseViewLocation(
      `#view=execution&project=%20%20&panel=${long}&item=a%00b`,
    );
    expect(parsed).toEqual({
      view: 'execution',
      project: null,
      panel: null,
      item: null,
    });
    expect(
      formatViewLocation({
        view: 'execution',
        project: long,
        panel: 'ab',
        item: '  ',
      }),
    ).toBe('view=execution');
  });

  it('默认位置序列化成空串，首屏不给 URL 添噪；其余往返不变', () => {
    expect(formatViewLocation(defaultViewLocation)).toBe('');
    const location = {
      view: 'operations' as const,
      project: '项目/资料夹',
      panel: null,
      item: null,
    };
    expect(parseViewLocation(`#${formatViewLocation(location)}`)).toEqual(
      location,
    );
  });

  it('同一位置判等按四格逐格比对', () => {
    const left = {
      view: 'documents' as const,
      project: null,
      panel: 'tree',
      item: null,
    };
    expect(sameViewLocation(left, { ...left })).toBe(true);
    expect(sameViewLocation(left, { ...left, panel: null })).toBe(false);
    expect(sameViewLocation(left, { ...left, view: 'inbox' })).toBe(false);
  });
});
