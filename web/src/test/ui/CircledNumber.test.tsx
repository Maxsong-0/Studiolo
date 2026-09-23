import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { CircledNumber } from '../../ui';

it('filled 为红色实心，label 进 aria-label 并作为图形的名字', () => {
  render(<CircledNumber n={1} filled label="待办 1，已在拓扑上标出" />);
  const mark = screen.getByRole('img', { name: '待办 1，已在拓扑上标出' });
  expect(mark).toHaveAttribute('aria-label', '待办 1，已在拓扑上标出');
  expect(mark).toHaveAttribute('data-filled', 'true');
  expect(mark).toHaveClass('ui-circled--filled');
  expect(mark).toHaveTextContent('1');
});

it('缺省黑色空心；不给 label 时就是圈里的数字', () => {
  const { container } = render(<CircledNumber n={4} />);
  const mark = container.firstElementChild as HTMLElement;
  expect(mark).toHaveAttribute('data-filled', 'false');
  expect(mark).not.toHaveClass('ui-circled--filled');
  expect(mark).not.toHaveAttribute('aria-label');
  expect(mark).not.toHaveAttribute('role');
  expect(screen.queryByRole('img')).toBeNull();
  expect(screen.getByText('4')).toBe(mark);
});
