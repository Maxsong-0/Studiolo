import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { ArchitecturePanel } from '../components/ArchitecturePanel';
import { architectureData, architectureLocation } from './architectureFixtures';
import { operationsData } from './operationsFixtures';

const { renderView } = vi.hoisted(() => ({ renderView: vi.fn() }));
vi.mock('../components/ArchitectureView', () => ({
  ArchitectureView: renderView,
}));
beforeEach(() => {
  renderView.mockReset().mockImplementation(() => {
    throw new Error('<img>图形渲染失败');
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

it('架构渲染失败保留三类纯文本声明、失败原因及各类来源', async () => {
  const architecture = architectureData();
  architecture.nodes[0].label = '<svg onload="alert(1)">接口';
  const onOpen = vi.fn();
  render(
    <ArchitecturePanel
      data={operationsData({ architecture })}
      onOpen={onOpen}
    />,
  );
  const fallback = screen.getByRole('region', { name: '架构声明文本数据' });
  expect(screen.getByRole('alert')).toHaveTextContent('<img>图形渲染失败');
  expect(fallback).toHaveTextContent('索引反馈自环');
  expect(fallback).toHaveTextContent('步骤仅表示设计顺序');
  expect(fallback.querySelector('img, svg, script')).toBeNull();
  await userEvent.click(
    within(fallback).getByRole('button', {
      name: '<svg onload="alert(1)">接口，从文本查看节点来源',
    }),
  );
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(7));
  await userEvent.click(
    within(fallback).getAllByRole('button', { name: '查看关系原文' })[0],
  );
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(16));
  await userEvent.click(
    within(fallback).getAllByRole('button', { name: '查看步骤原文' })[0],
  );
  expect(onOpen).toHaveBeenLastCalledWith(architectureLocation(24));
});

it('切换文档代次后可重新呈现架构图', () => {
  const data = operationsData({ architecture: architectureData() });
  const { rerender } = render(
    <ArchitecturePanel data={data} onOpen={vi.fn()} />,
  );
  expect(
    screen.getByRole('region', { name: '架构声明文本数据' }),
  ).toBeInTheDocument();
  renderView.mockImplementation(() => <p>架构图恢复</p>);
  rerender(
    <ArchitecturePanel data={{ ...data, generation: 8 }} onOpen={vi.fn()} />,
  );
  expect(screen.getByText('架构图恢复')).toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: '架构声明文本数据' }),
  ).not.toBeInTheDocument();
});
