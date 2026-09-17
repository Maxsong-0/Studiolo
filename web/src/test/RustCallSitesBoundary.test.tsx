import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { RustCallSitesPanel } from '../components/RustCallSitesPanel';
import { operationsData } from './operationsFixtures';
import { rustData, rustGraph } from './rustCallFixtures';

const { renderGraph } = vi.hoisted(() => ({ renderGraph: vi.fn() }));
vi.mock('../components/RustCallSitesView', () => ({
  RustCallSitesView: renderGraph,
}));
beforeEach(() => {
  renderGraph.mockReset().mockImplementation(() => {
    throw new Error('<img>渲染失败');
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

it('渲染失败退回带全部文件、上下文、调用位置和字节列的纯文本', () => {
  const graph = rustGraph();
  graph.calls[0].name = '<svg onload="alert(1)">';
  render(
    <RustCallSitesPanel
      data={operationsData({ rust_call_sites: rustData({ graph }) })}
      onOpen={vi.fn()}
    />,
  );
  const fallback = screen.getByRole('region', {
    name: 'Rust 调用位置文本数据',
  });
  expect(screen.getByRole('alert')).toHaveTextContent('<img>渲染失败');
  expect(fallback).toHaveTextContent('src/empty.rs');
  expect(fallback).toHaveTextContent('闭包 · 匿名');
  expect(fallback).toHaveTextContent('f0:context:70:110');
  expect(fallback).toHaveTextContent('第 3 行 · UTF-8 字节列 13');
  expect(fallback).toHaveTextContent('未展开宏调用');
  expect(fallback).toHaveTextContent('<svg onload="alert(1)">');
  expect(fallback.querySelector('img, svg, script, button')).toBeNull();
});

it.each(['project', 'generation', 'observed'] as const)(
  '换%s重建错误边界',
  (change) => {
    const data = operationsData({ rust_call_sites: rustData() });
    const { rerender } = render(
      <RustCallSitesPanel data={data} onOpen={vi.fn()} />,
    );
    expect(
      screen.getByRole('region', { name: 'Rust 调用位置文本数据' }),
    ).toBeInTheDocument();
    renderGraph.mockImplementation(() => <p>源码位置恢复</p>);
    if (change === 'project') data.project = '项目/乙';
    if (change === 'generation') data.generation = 8;
    if (change === 'observed')
      data.rust_call_sites = rustData({ observed_at: '1789140300' });
    rerender(<RustCallSitesPanel data={data} onOpen={vi.fn()} />);
    expect(screen.getByText('源码位置恢复')).toBeInTheDocument();
    expect(
      screen.queryByRole('region', { name: 'Rust 调用位置文本数据' }),
    ).not.toBeInTheDocument();
  },
);
