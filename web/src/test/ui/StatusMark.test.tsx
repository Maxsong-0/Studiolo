import { render, screen } from '@testing-library/react';
import { expect, it } from 'vitest';
import { StatusMark, type StatusState } from '../../ui';

const cases: [StatusState, string][] = [
  ['ok', '已上线'],
  ['active', '运行中'],
  ['pending', '开发中'],
  ['alert', '生产不可达'],
  ['dormant', '休眠'],
];

it.each(cases)('%s 状态的文字始终显示，方块只是辅助', (state, text) => {
  const { container } = render(<StatusMark state={state}>{text}</StatusMark>);
  const root = container.firstElementChild as HTMLElement;
  expect(root).toHaveAttribute('data-state', state);
  expect(screen.getByText(text)).toBeVisible();
  const mark = root.querySelector('.ui-status__mark');
  expect(mark).toHaveAttribute('aria-hidden', 'true');
  expect(mark).toBeEmptyDOMElement();
  expect(root).toHaveTextContent(new RegExp(`^${text}$`));
});
