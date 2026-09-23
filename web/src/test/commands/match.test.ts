import { expect, it } from 'vitest';
import {
  highlightRanges,
  matchCommand,
  queryTerms,
} from '../../commands/match';

it('输入按任意空白切段并转小写；全是空白时没有片段', () => {
  expect(queryTerms('  协作　时间线  POLICY ')).toEqual([
    '协作',
    '时间线',
    'policy',
  ]);
  expect(queryTerms(' \t ')).toEqual([]);
});

it('每段都要在名字、说明或别名里出现；名字开头命中得分最低', () => {
  const command = {
    label: '刷新快照',
    hint: '重新读取工作台',
    keywords: ['refresh'],
  };
  expect(matchCommand(command, [])).toEqual({ score: 0, ranges: [] });
  expect(matchCommand(command, ['刷新'])).toEqual({
    score: 0,
    ranges: [[0, 2]],
  });
  expect(matchCommand(command, ['快照'])).toEqual({
    score: 1,
    ranges: [[2, 4]],
  });
  expect(matchCommand(command, ['快照', '工作台'])).toEqual({
    score: 2,
    ranges: [[2, 4]],
  });
  expect(matchCommand(command, ['REFRESH'.toLowerCase()])).toEqual({
    score: 2,
    ranges: [],
  });
  expect(matchCommand(command, ['快照', '没有'])).toBeNull();
});

it('标线区间按位置排好、重叠的合并；小写化改变长度时不标', () => {
  expect(highlightRanges('Studiolo 协作 Studio', ['studio', 'dio'])).toEqual([
    [0, 6],
  ]);
  expect(highlightRanges('协作 › 时间线', ['时间', '协作'])).toEqual([
    [0, 2],
    [5, 7],
  ]);
  // 「İ」小写后变成两个码元，下标对不上原文。
  expect(highlightRanges('İstanbul', ['stan'])).toEqual([]);
});
