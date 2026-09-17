import { render, screen } from '@testing-library/react';
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { CargoDependencyPanel } from '../components/CargoDependencyPanel';
import { cargoData, cargoGraph } from './cargoFixtures';
import { operationsData } from './operationsFixtures';

const { renderGraph } = vi.hoisted(() => ({ renderGraph: vi.fn() }));
vi.mock('../components/CargoDependencyView', () => ({
  CargoDependencyView: renderGraph,
}));
beforeEach(() => {
  renderGraph.mockReset().mockImplementation(() => {
    throw new Error('<img>渲染失败');
  });
  vi.spyOn(console, 'error').mockImplementation(() => {});
});
afterEach(() => vi.restoreAllMocks());

it('图形失败保留清单包、方向、条件与全部来源的只读文本', () => {
  const graph = cargoGraph();
  graph.dependencies[0].target = '<svg onload="alert(1)">';
  render(
    <CargoDependencyPanel
      data={operationsData({ cargo_dependencies: cargoData({ graph }) })}
      onOpen={vi.fn()}
    />,
  );
  const fallback = screen.getByRole('region', {
    name: 'Cargo 清单依赖文本数据',
  });
  expect(screen.getByRole('alert')).toHaveTextContent('<img>渲染失败');
  expect(fallback).toHaveTextContent('app → core');
  expect(fallback).toHaveTextContent('继承定义来源：Cargo.toml · 第 40 行');
  expect(fallback).toHaveTextContent('crates/core/Cargo.toml · 第 3 行');
  expect(fallback).toHaveTextContent('<svg onload="alert(1)">');
  expect(fallback.querySelector('img, svg, script, button')).toBeNull();
});

it('新文档代次重建错误边界并重新显示图形', () => {
  const data = operationsData({ cargo_dependencies: cargoData() });
  const { rerender } = render(
    <CargoDependencyPanel data={data} onOpen={vi.fn()} />,
  );
  expect(
    screen.getByRole('region', { name: 'Cargo 清单依赖文本数据' }),
  ).toBeInTheDocument();
  renderGraph.mockImplementation(() => <p>清单图恢复</p>);
  rerender(
    <CargoDependencyPanel data={{ ...data, generation: 8 }} onOpen={vi.fn()} />,
  );
  expect(screen.getByText('清单图恢复')).toBeInTheDocument();
  expect(
    screen.queryByRole('region', { name: 'Cargo 清单依赖文本数据' }),
  ).not.toBeInTheDocument();
});
