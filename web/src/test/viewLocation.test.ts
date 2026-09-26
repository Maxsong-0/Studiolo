import { describe, expect, it } from 'vitest';
import type { View } from '../types';
import {
  defaultViewLocation,
  formatViewLocation,
  parseViewLocation,
  sameViewLocation,
} from '../viewLocation';

const project = '项目/星舟';
const encodedProject = encodeURIComponent(project);

describe('视图位置', () => {
  it('解析视图与三格导航状态，`#` 可有可无', () => {
    const expected = {
      view: 'collab' as const,
      project,
      panel: 'execution',
      item: 'T-001',
    };
    expect(
      parseViewLocation(
        `#view=collab&project=${encodedProject}&panel=execution&item=T-001`,
      ),
    ).toEqual(expected);
    expect(
      parseViewLocation(
        `view=collab&project=${encodedProject}&panel=execution&item=T-001`,
      ),
    ).toEqual(expected);
  });

  it('未知视图、空 hash 与损坏内容一律回落驾驶舱，不抛错也不保留子键', () => {
    for (const hash of [
      '',
      '#',
      '#view=',
      `#view=unknown&project=${encodedProject}`,
      '#%%%',
      `#project=${encodedProject}`,
      // 原型链上的名字不能被当成旧视图去迁移。
      `#view=constructor&project=${encodedProject}`,
      '#view=__proto__',
      '#view=toString&panel=tree',
      '#view=hasOwnProperty',
      // 视图名区分大小写，也不接受首尾空白。
      '#view=Overview',
      '#view=BOARD',
      '#view=%20board',
    ])
      expect(parseViewLocation(hash)).toEqual(defaultViewLocation);
  });

  it('空值、超长值与控制字符不算位置，不把 URL 里的外部输入原样回显', () => {
    const long = 'a'.repeat(513);
    expect(
      parseViewLocation(`#view=collab&project=%20%20&panel=${long}&item=a%00b`),
    ).toEqual({ view: 'collab', project: null, panel: null, item: null });
    expect(
      formatViewLocation({
        view: 'collab',
        project: long,
        panel: 'a\u0007b',
        item: '  ',
      }),
    ).toBe('view=collab');
  });

  it('默认位置序列化成空串，首屏不给 URL 添噪；其余往返不变', () => {
    expect(formatViewLocation(defaultViewLocation)).toBe('');
    const location = {
      view: 'projects' as const,
      project: '项目/资料夹',
      panel: 'operations',
      item: null,
    };
    expect(parseViewLocation(`#${formatViewLocation(location)}`)).toEqual(
      location,
    );
  });

  it('新六视图都能原样往返', () => {
    for (const view of [
      'cockpit',
      'projects',
      'documents',
      'collab',
      'rules',
      'policy',
    ] as const) {
      const location = { view, project: project, panel: 'p', item: 'i' };
      expect(parseViewLocation(`#${formatViewLocation(location)}`)).toEqual(
        location,
      );
    }
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
    expect(sameViewLocation(left, { ...left, view: 'cockpit' })).toBe(false);
  });
});

describe('旧链接迁移（设计规范第 4 节）', () => {
  it('view=overview 与空 hash 落到驾驶舱；旧总览没用过的格子不带过去', () => {
    expect(parseViewLocation('#view=overview')).toEqual(defaultViewLocation);
    expect(
      parseViewLocation(
        `#view=overview&project=${encodedProject}&panel=x&item=y`,
      ),
    ).toEqual(defaultViewLocation);
    expect(parseViewLocation('')).toEqual(defaultViewLocation);
  });

  it('view=inbox 落到驾驶舱（首格即待我动作）', () => {
    expect(parseViewLocation('#view=inbox')).toEqual(defaultViewLocation);
    expect(
      parseViewLocation(`#view=inbox&project=${encodedProject}&item=A-1`),
    ).toEqual(defaultViewLocation);
  });

  it('view=operations&project=P&panel=X 落到项目运作，旧 panel 值原样放进 item', () => {
    expect(
      parseViewLocation(
        `#view=operations&project=${encodedProject}&panel=cargo`,
      ),
    ).toEqual({
      view: 'projects',
      project,
      panel: 'operations',
      item: 'cargo',
    });
    // 没有旧 panel 就没有 item；旧运作视图从不写 item，带进来的 item 不冒充面板。
    expect(
      parseViewLocation(`#view=operations&project=${encodedProject}`),
    ).toEqual({
      view: 'projects',
      project,
      panel: 'operations',
      item: null,
    });
    expect(
      parseViewLocation(
        `#view=operations&project=${encodedProject}&panel=rust&item=stale`,
      ),
    ).toEqual({
      view: 'projects',
      project,
      panel: 'operations',
      item: 'rust',
    });
  });

  it('view=board&project=P 落到协作的任务与回执', () => {
    expect(parseViewLocation(`#view=board&project=${encodedProject}`)).toEqual({
      view: 'collab',
      project,
      panel: 'board',
      item: null,
    });
    // 旧黑板只认 replay：别的 panel 值仍是实时黑板，旧黑板从不写 item。
    expect(
      parseViewLocation(`#view=board&project=${encodedProject}&panel=x&item=y`),
    ).toEqual({ view: 'collab', project, panel: 'board', item: null });
  });

  it('view=board&panel=replay&project=P 落到协作的时间线', () => {
    expect(
      parseViewLocation(`#view=board&panel=replay&project=${encodedProject}`),
    ).toEqual({ view: 'collab', project, panel: 'timeline', item: null });
  });

  it('view=execution&project=P&item=I 落到协作的执行，条目原样保留', () => {
    expect(
      parseViewLocation(
        `#view=execution&project=${encodedProject}&item=01J000TASK`,
      ),
    ).toEqual({
      view: 'collab',
      project,
      panel: 'execution',
      item: '01J000TASK',
    });
    // 旧执行视图不用 panel：面板一律是执行。
    expect(parseViewLocation('#view=execution&panel=replay')).toEqual({
      view: 'collab',
      project: null,
      panel: 'execution',
      item: null,
    });
  });

  it('旧链接里的损坏格子照样丢弃，迁移不绕过校验', () => {
    const long = 'a'.repeat(513);
    expect(
      parseViewLocation(
        `#view=execution&project=%20%20&panel=${long}&item=a%00b`,
      ),
    ).toEqual({
      view: 'collab',
      project: null,
      panel: 'execution',
      item: null,
    });
    expect(
      parseViewLocation(`#view=operations&project=${long}&panel=a%07b`),
    ).toEqual({
      view: 'projects',
      project: null,
      panel: 'operations',
      item: null,
    });
    expect(parseViewLocation('#view=board&project=%00&panel=replay')).toEqual({
      view: 'collab',
      project: null,
      panel: 'timeline',
      item: null,
    });
  });

  it('迁移后写回 URL 只用新名；旧名传进格式化一律不写', () => {
    expect(
      formatViewLocation(
        parseViewLocation(`#view=board&panel=replay&project=${encodedProject}`),
      ),
    ).toBe(`view=collab&project=${encodedProject}&panel=timeline`);
    expect(
      formatViewLocation(
        parseViewLocation(
          `#view=operations&project=${encodedProject}&panel=cargo`,
        ),
      ),
    ).toBe(
      `view=projects&project=${encodedProject}&panel=operations&item=cargo`,
    );
    expect(formatViewLocation(parseViewLocation('#view=inbox'))).toBe('');
    for (const legacy of [
      'overview',
      'inbox',
      'operations',
      'board',
      'execution',
    ])
      expect(
        formatViewLocation({
          view: legacy as View,
          project,
          panel: null,
          item: null,
        }),
      ).toBe('');
  });
});
