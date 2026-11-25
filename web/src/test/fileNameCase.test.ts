import { readdirSync } from 'node:fs';
import { join, relative, sep } from 'node:path';
import { describe, expect, it } from 'vitest';

/**
 * 文件名大小写撞名守卫（T-232）。
 * Mac 默认的文件系统不分大小写：`Ago.tsx` 和 `ago.ts` 在 Linux 上是两个文件，在 Mac 上 `import './Ago'`
 * 会先试 `Ago.ts`、被认成 `ago.ts`，开发服务白屏（缺导出）。测试箱是 Linux，抓不到，所以在这里钉住：
 * web/ 下任意两个文件，去掉最后一个扩展名后只差大小写，就算撞名。
 * 修法：把其中一个改成说明内容的名字（组件叫 `Ago.tsx`，纯函数就叫 `agoLabel.ts`），不要靠大小写区分。
 */

const WEB = join(import.meta.dirname, '../..');
/** 不是我们的源码：依赖和构建产物。 */
const SKIPPED_DIRS = new Set(['node_modules', 'dist', 'coverage', '.git']);

/** 去掉最后一个扩展名；`.gitignore` 这类点开头的名字整个算文件名。 */
const stemOf = (path: string) => path.replace(/(?<=[^/])\.[^./]+$/, '');

/**
 * 返回每一组撞名的文件（相对路径，组内排序）；纯函数，便于造正反例。
 * 撞名 = 小写后同名、但原样并不同名。`x.css` 和 `x.tsx` 原样同名，是正常搭配，不算。
 */
export function findCaseCollisions(paths: string[]): string[][] {
  const groups = new Map<string, string[]>();
  for (const path of paths) {
    const key = stemOf(path).toLowerCase();
    groups.set(key, [...(groups.get(key) ?? []), path]);
  }
  return [...groups.values()]
    .filter((group) => new Set(group.map(stemOf)).size > 1)
    .map((group) => group.sort());
}

function listFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    if (entry.isDirectory()) {
      return SKIPPED_DIRS.has(entry.name)
        ? []
        : listFiles(join(dir, entry.name));
    }
    return [relative(WEB, join(dir, entry.name)).split(sep).join('/')];
  });
}

function explain(groups: string[][]): string {
  return (
    `这些文件的名字去掉扩展名后只差大小写：\n${groups.map((g) => `  ${g.join('  ↔  ')}`).join('\n')}\n` +
    '在不分大小写的文件系统（Mac）上它们会互相顶替，`vite dev` 白屏。' +
    '把其中一个（通常是不含组件的那个）改成说明内容的名字，并同步改所有 import；不要靠大小写区分。'
  );
}

describe('文件名大小写撞名', () => {
  it('web/ 下没有只差大小写的两个文件', () => {
    const groups = findCaseCollisions(listFiles(WEB));
    expect(groups, explain(groups)).toEqual([]);
  });

  it('反例：同目录只差大小写、扩展名不同的也算撞名', () => {
    expect(
      findCaseCollisions(['src/live/Ago.tsx', 'src/live/ago.ts', 'src/a.ts']),
    ).toEqual([['src/live/Ago.tsx', 'src/live/ago.ts']]);
    expect(
      findCaseCollisions([
        'src/test/CapabilityDiff.test.tsx',
        'src/test/capabilityDiff.test.ts',
      ]),
    ).toHaveLength(1);
  });

  it('正例：名字不同、不同目录、同名不同扩展名都不算撞名', () => {
    expect(
      findCaseCollisions([
        'src/live/Ago.tsx',
        'src/live/agoLabel.ts',
        'src/a/Ago.tsx',
        'src/b/ago.ts',
        'src/x.css',
        'src/x.tsx',
        '.gitignore',
        '.prettierignore',
      ]),
    ).toEqual([]);
  });
});
