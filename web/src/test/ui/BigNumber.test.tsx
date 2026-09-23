import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { BigNumber } from '../../ui';

it('按 digits 补零，读屏读自然数', () => {
  const { container } = render(
    <BigNumber value={5} label="件事等你处理" note="最久 5 天" />,
  );
  const shown = screen.getByText('05');
  expect(shown).toHaveAttribute('aria-hidden', 'true');
  expect(container.querySelector('.sr-only')).toHaveTextContent(/^5$/);
  expect(screen.getByText('件事等你处理')).toBeInTheDocument();
  expect(screen.getByText('最久 5 天')).toBeInTheDocument();
});

it.each([
  [7, 3, '007', '7'],
  [0, 2, '00', '0'],
  [-3, 2, '-03', '-3'],
  [12, 2, '12', null],
  [123, 2, '123', null],
  [2.5, 2, '2.5', null],
])(
  'value=%s digits=%s 显示 %s，读屏读 %s（null＝与显示相同，不重复一份）',
  (value, digits, shown, spoken) => {
    const { container } = render(
      <BigNumber value={value} digits={digits} label="读数" />,
    );
    const visible = screen.getByText(shown);
    const hidden = container.querySelector('.sr-only');
    if (spoken === null) {
      expect(visible).not.toHaveAttribute('aria-hidden');
      expect(hidden).toBeNull();
    } else {
      expect(visible).toHaveAttribute('aria-hidden', 'true');
      expect(hidden).toHaveTextContent(new RegExp(`^${spoken}$`));
    }
  },
);

it('null 与非有限数显示「—」，不补任何数', () => {
  const { rerender, container } = render(
    <BigNumber value={null} label="件事等你处理" />,
  );
  expect(screen.getByText('—')).toBeInTheDocument();
  expect(container.querySelector('.sr-only')).toBeNull();
  expect(container).not.toHaveTextContent(/\d/);

  rerender(<BigNumber value={Number.NaN} label="件事等你处理" />);
  expect(screen.getByText('—')).toBeInTheDocument();
  expect(container).not.toHaveTextContent(/\d/);
});
