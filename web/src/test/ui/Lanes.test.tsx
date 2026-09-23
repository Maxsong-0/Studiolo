import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { Lanes, type Lane } from '../../ui';
import { LANE_MIN_WIDTH, laneBoxes } from '../../ui/geometry';

const lanes: Lane[] = [
  {
    name: 'CLAUDE',
    blocks: [
      { start: 6, end: 9, tone: 'work' },
      { start: 13.2, end: 15.6, tone: 'work' },
      { start: 21, end: 22, tone: 'work' },
    ],
  },
  {
    name: '你',
    blocks: [
      { start: 11.7, end: 11.72, tone: 'owner' },
      { start: 19.99, end: 20.5, tone: 'owner' },
      { start: 12, end: 12, tone: 'owner' },
      { start: 10, end: 9, tone: 'owner' },
      { start: Number.NaN, end: 12, tone: 'owner' },
    ],
  },
];
const ticks = ['08:00', '11:00', '14:00', '17:00', '20:00'];

const boxesOf = (lane: string) => {
  const name = screen.getByText(lane);
  const track = name.nextElementSibling as HTMLElement;
  return Array.from(track.querySelectorAll('rect')).map((rect) => ({
    x: rect.getAttribute('x'),
    width: rect.getAttribute('width'),
    owner: rect.classList.contains('ui-lanes__block--owner'),
  }));
};

it('超出范围的块裁剪到轨道内，整块在外的不画', () => {
  render(<Lanes from={8} to={20} lanes={lanes} ticks={ticks} />);
  expect(boxesOf('CLAUDE')).toEqual([
    { x: '0', width: '8.333', owner: false },
    { x: '43.333', width: '20', owner: false },
  ]);
});

it('过窄的块按最小宽度 0.6% 画，贴右缘时向左让位；首尾颠倒或非数的块不画', () => {
  expect(LANE_MIN_WIDTH).toBe(0.6);
  render(<Lanes from={8} to={20} lanes={lanes} ticks={ticks} />);
  expect(boxesOf('你')).toEqual([
    { x: '30.833', width: '0.6', owner: true },
    // 19.99–20 只有 0.083%：按 0.6% 画，左缘从 99.917 让到 99.4，不越出轨道
    { x: '99.4', width: '0.6', owner: true },
    { x: '33.333', width: '0.6', owner: true },
  ]);
});

it('整张图是 role=img，按裁剪后的块生成文字摘要；刻度照原样显示', () => {
  render(<Lanes from={8} to={20} lanes={lanes} ticks={ticks} />);
  expect(
    screen.getByRole('img', {
      name: '时间泳道 08:00–20:00；CLAUDE：工作 2 段；你：审批 3 段',
    }),
  ).toBeInTheDocument();
  for (const tick of ticks) expect(screen.getByText(tick)).toBeInTheDocument();
});

it('给了 label 就用 label；空泳道摘要写「无」', () => {
  const { rerender } = render(
    <Lanes from={0} to={10} lanes={[{ name: 'CODEX', blocks: [] }]} />,
  );
  expect(
    screen.getByRole('img', { name: '时间泳道；CODEX：无' }),
  ).toBeInTheDocument();
  rerender(
    <Lanes
      from={0}
      to={10}
      lanes={[{ name: 'CODEX', blocks: [] }]}
      label="今天的时间线"
    />,
  );
  expect(screen.getByRole('img', { name: '今天的时间线' })).toBeInTheDocument();
});

it('边界：刚好贴着范围的正长度块不画，范围端点上的瞬时块要画，范围无效时一块不画', () => {
  expect(
    laneBoxes(8, 20, [
      { start: 7, end: 8, tone: 'work' },
      { start: 20, end: 21, tone: 'work' },
      { start: 8, end: 8, tone: 'owner' },
      { start: 20, end: 20, tone: 'owner' },
    ]),
  ).toEqual([
    { left: 0, width: 0.6, tone: 'owner' },
    { left: 99.4, width: 0.6, tone: 'owner' },
  ]);
  expect(laneBoxes(8, 20, [{ start: 0, end: 100, tone: 'work' }])).toEqual([
    { left: 0, width: 100, tone: 'work' },
  ]);
  expect(laneBoxes(20, 8, [{ start: 9, end: 10, tone: 'work' }])).toEqual([]);
  expect(laneBoxes(8, 8, [{ start: 8, end: 8, tone: 'work' }])).toEqual([]);
  expect(
    laneBoxes(8, Number.POSITIVE_INFINITY, [
      { start: 9, end: 10, tone: 'work' },
    ]),
  ).toEqual([]);
});
