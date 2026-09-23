import {
  act,
  configure,
  fireEvent,
  render,
  screen,
  within,
} from '@testing-library/react';
import { describe, expect, it } from 'vitest';
import { createFixtureApi } from '../../../dev/fixtureApi';
import { TASK } from '../../../dev/fixtures/ids';
import { parseBoardOverview } from '../../boardOverviewModel';
import { TopologyDiagram } from '../../collab/TopologyDiagram';
import { parseViewLocation } from '../../viewLocation';

// 同 CollabView.test.tsx：jsdom 不加载样式表，角色查询不必逐节点判可见性。
configure({ defaultHidden: true });

const NOW = new Date(2026, 8, 22, 20, 31).getTime();

function overview() {
  const api = createFixtureApi({ now: () => NOW });
  return parseBoardOverview(
    api.handle({
      method: 'GET',
      url: '/api/v1/board/overview?window_hours=24',
      headers: {},
    }).body,
    24,
  );
}

describe('TopologyDiagram', { timeout: 30_000 }, () => {
  it('mini：缺省不画休眠，只画 maxTasks 条（圈号指到的优先），另给「另 N 条，去协作页」', async () => {
    const data = overview();
    window.location.hash = '#view=cockpit';
    render(
      <TopologyDiagram
        items={data.items}
        mode="mini"
        maxTasks={3}
        markers={[
          { n: 1, target: { taskId: TASK.T108 } },
          { n: 2, target: { path: 'deploy/human/offline' } },
        ]}
        title="协作拓扑，今天"
        now={Date.parse(data.generatedAt)}
      />,
    );
    const figure = screen.getByRole('img', { name: '协作拓扑，今天' });
    expect(
      [...figure.querySelectorAll('[data-task-id]')].map((node) =>
        node.getAttribute('data-task-id'),
      ),
    ).toEqual([TASK.T093, TASK.T106, TASK.T108]);
    const markers = [...figure.querySelectorAll('.topology__marker')];
    expect(
      markers.map((node) => node.querySelector('title')?.textContent),
    ).toEqual([
      '待办 1：任务「T-108 副本扩容纳入 Semiment-Docs」',
      '待办 2：路径 deploy/human/offline',
    ]);
    expect(
      markers.map((node) => node.querySelector('circle')?.getAttribute('r')),
    ).toEqual(['11', '11']);
    // 没给 onSelectTask：清单只列文字，不给按钮
    const list = screen.getByRole('list', {
      name: '协作拓扑，今天：按执行者列出',
    });
    expect(within(list).queryAllByRole('button')).toHaveLength(0);
    expect(list).toHaveTextContent('待办 1');
    await act(async () => {
      fireEvent.click(
        screen.getByRole('button', { name: '另 3 条，去协作页' }),
      );
    });
    expect(parseViewLocation(window.location.hash)).toEqual({
      view: 'collab',
      project: null,
      panel: null,
      item: null,
    });
  });

  it('full：路径文字进清单，重叠用红字写出；viewBox 宽 1030、高随行数', () => {
    const data = overview();
    render(
      <TopologyDiagram
        items={data.items}
        mode="full"
        title="协作拓扑"
        now={Date.parse(data.generatedAt)}
        onSelectTask={() => {}}
      />,
    );
    const figure = screen.getByRole('img', { name: '协作拓扑' });
    const [, , width, height] = (figure.getAttribute('viewBox') ?? '')
      .split(' ')
      .map(Number);
    expect(width).toBe(1030);
    expect(height).toBeGreaterThan(400);
    const list = screen.getByRole('list', { name: '协作拓扑：按执行者列出' });
    expect(within(list).getAllByRole('button')).toHaveLength(7);
    expect(list).toHaveTextContent(
      '写域重叠：包含于 web（T-130 前端全面重设计 · 进行中）',
    );
    expect(list).toHaveTextContent('Codex · 01M23休眠 · 1 条');
  });

  it('没有未结束的任务时给空态，不画空图', () => {
    render(<TopologyDiagram items={[]} mode="full" title="协作拓扑" />);
    expect(screen.getByText('黑板上没有未结束的任务')).toBeInTheDocument();
    expect(screen.queryByRole('img')).not.toBeInTheDocument();
  });
});
