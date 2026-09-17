import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { RustCallSitesPanel } from '../components/RustCallSitesPanel';
import { operationsData } from './operationsFixtures';
import { rustAnalysis } from './rustCallLinksFixtures';

const { renderGraph } = vi.hoisted(() => ({ renderGraph: vi.fn() }));
vi.mock('../components/RustCallSitesView', () => ({
  RustCallSitesView: renderGraph,
}));
beforeEach(() => {
  renderGraph.mockReset().mockImplementation(() => {
    throw new Error('<img>关联渲染失败');
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

it('文本降级保留正向定义来源、反向引用和未解析原因', () => {
  const rust = rustAnalysis();
  rust.graph!.calls[2].name = '<svg onload=alert(1)>';
  render(
    <RustCallSitesPanel
      data={operationsData({ rust_call_sites: rust })}
      onOpen={vi.fn()}
    />,
  );
  const text = screen.getByRole('region', { name: 'Rust 调用位置文本数据' });
  expect(text).toHaveTextContent('词法关联的同文件函数定义：读取');
  expect(text).toHaveTextContent('第 8 行 · UTF-8 字节列 1');
  expect(text).toHaveTextContent('同批词法关联引用：3 处');
  expect(text).toHaveTextContent('当前允许的同文件范围内没有找到函数定义');
  expect(text).toHaveTextContent('<svg onload=alert(1)>');
  expect(text.querySelector('img, svg, script, button')).toBeNull();
  expect(screen.getByRole('alert')).toHaveTextContent('<img>关联渲染失败');
});

it('同秒新批重建渲染边界，不保留旧批失败态', () => {
  const data = operationsData({ rust_call_sites: rustAnalysis() });
  const { rerender } = render(
    <RustCallSitesPanel data={data} onOpen={vi.fn()} />,
  );
  expect(screen.getByRole('alert')).toBeInTheDocument();
  renderGraph.mockImplementation(() => <p>新批关联可见</p>);
  const rust = rustAnalysis();
  rust.links!.batch_id = 'c'.repeat(64);
  rerender(
    <RustCallSitesPanel
      data={{ ...data, rust_call_sites: rust }}
      onOpen={vi.fn()}
    />,
  );
  expect(screen.getByText('新批关联可见')).toBeInTheDocument();
  expect(screen.queryByRole('alert')).not.toBeInTheDocument();
});
