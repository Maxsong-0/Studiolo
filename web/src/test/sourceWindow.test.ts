import { describe, expect, it } from 'vitest';
import { sourceWindow } from '../sourceWindow';

describe('大文档原文降级', () => {
  it('普通文档保留所有行和准确行号', () => {
    expect(sourceWindow('第一行\r\n第二行\n', 2)).toEqual({
      large: false,
      start: 1,
      end: 3,
      lines: [
        { number: 1, text: '第一行' },
        { number: 2, text: '第二行' },
        { number: 3, text: '' },
      ],
    });
  });

  it('按 UTF-8 字节阈值降级，目标前后至多 100 行，原始行号不重置', () => {
    const content = Array.from(
      { length: 2000 },
      (_, index) => `${index + 1} ${'原文'.repeat(30)}`,
    ).join('\n');
    const window = sourceWindow(content, 1000);
    expect(content.length).toBeLessThan(256 * 1024);
    expect(window.large).toBe(true);
    expect(window.start).toBe(900);
    expect(window.end).toBe(1100);
    expect(window.lines).toHaveLength(201);
    expect(window.lines[100].number).toBe(1000);
    expect(window.lines[100].text).toMatch(/^1000 /);
  });
});
