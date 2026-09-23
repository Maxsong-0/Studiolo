import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { Sparkbars } from '../../ui';
import { sparkbarsGeometry } from '../../ui/geometry';

const label = '最近六轮同步耗时，限值 120 秒';

it('根元素 role=img，aria-label 就是 label', () => {
  render(<Sparkbars values={[10, 60]} limit={120} label={label} />);
  const chart = screen.getByRole('img', { name: label });
  expect(chart.tagName.toLowerCase()).toBe('svg');
  expect(chart).toHaveAttribute('aria-label', label);
});

it('限值画成一条横贯的红色虚线，位置按纵轴换算', () => {
  const { container } = render(
    <Sparkbars
      values={[10, 60, 0, -5, Number.NaN, 150]}
      limit={120}
      label={label}
    />,
  );
  const limit = container.querySelector('.ui-sparkbars__limit');
  expect(limit).not.toBeNull();
  // 纵轴上限取 150：64 - 120/150 × (64 - 2) = 14.4
  expect(limit).toHaveAttribute('y1', '14.4');
  expect(limit).toHaveAttribute('y2', '14.4');
  expect(limit).toHaveAttribute('x1', '0');
  expect(limit).toHaveAttribute('x2', '6');
  expect(limit).toHaveAttribute('vector-effect', 'non-scaling-stroke');
  expect(screen.getByRole('img')).toHaveAttribute('viewBox', '0 0 6 64');
});

it('零、负数与非数不画柱，其余柱高按比例；超过 max 的柱顶到顶', () => {
  const { container, rerender } = render(
    <Sparkbars
      values={[10, 60, 0, -5, Number.NaN, 150]}
      limit={120}
      label={label}
    />,
  );
  const tops = () =>
    Array.from(container.querySelectorAll('.ui-sparkbars__bar')).map((bar) => [
      bar.getAttribute('x1'),
      bar.getAttribute('y2'),
    ]);
  expect(tops()).toEqual([
    ['0.5', '59.867'],
    ['1.5', '39.2'],
    ['5.5', '2'],
  ]);

  rerender(
    <Sparkbars
      values={[10, 60, 0, -5, Number.NaN, 150]}
      limit={120}
      max={100}
      height={40}
      label={label}
    />,
  );
  expect(tops()).toEqual([
    ['0.5', '36.2'],
    ['1.5', '17.2'],
    ['5.5', '2'],
  ]);
  expect(container.querySelector('.ui-sparkbars__limit')).toHaveAttribute(
    'y1',
    '2',
  );
});

it('没有限值就没有虚线；全为零时柱都不画', () => {
  const { container } = render(<Sparkbars values={[0, 0]} label={label} />);
  expect(container.querySelector('.ui-sparkbars__limit')).toBeNull();
  expect(container.querySelectorAll('.ui-sparkbars__bar')).toHaveLength(0);
  expect(sparkbarsGeometry([], 64, null, null)).toEqual({
    width: 1,
    bars: [],
    limitY: null,
  });
  expect(sparkbarsGeometry([5], 64, -1, null).limitY).toBeNull();
});
