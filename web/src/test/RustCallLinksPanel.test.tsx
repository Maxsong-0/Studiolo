import { render, screen, within } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { RustCallSitesPanel } from '../components/RustCallSitesPanel';
import { rustUnresolvedReasons } from '../rustCallLinks';
import { operationsData } from './operationsFixtures';
import { rustAnalysis } from './rustCallLinksFixtures';
import { rustData } from './rustCallFixtures';

const inspect = '查看调用 读取 的词法关联，第 3 行字节列 5';
function mount(rust = rustAnalysis()) {
  const onOpen = vi.fn();
  render(
    <RustCallSitesPanel
      data={operationsData({ rust_call_sites: rust })}
      onOpen={onOpen}
    />,
  );
  return onOpen;
}

it('调用可用键盘查看同文件定义及全部同批反向引用，递归引用保留调用位置', async () => {
  const fetchMock = vi.fn();
  vi.stubGlobal('fetch', fetchMock);
  const onOpen = mount();
  const user = userEvent.setup();
  const button = screen.getByRole('button', { name: inspect });
  button.focus();
  await user.keyboard('{Enter}');
  const details = screen.getByRole('region', { name: 'Rust 词法关联详情' });
  expect(details).toHaveFocus();
  expect(details).toHaveTextContent('函数定义：读取');
  expect(details).toHaveTextContent('第 8 行 · UTF-8 字节列 1');
  const references = within(details).getByRole('list', {
    name: '函数的词法关联引用',
  });
  expect(within(references).getAllByRole('listitem')).toHaveLength(3);
  expect(references).toHaveTextContent('r#读取');
  await user.click(
    within(references).getByRole('button', {
      name: '查看引用 读取，第 9 行字节列 5',
    }),
  );
  expect(
    within(details).getByRole('heading', { name: '所选调用：读取' })
      .parentElement,
  ).toHaveTextContent('第 9 行 · UTF-8 字节列 5');
  expect(onOpen).not.toHaveBeenCalled();
  expect(fetchMock).not.toHaveBeenCalled();
});

it('自由函数可查看零引用，反查引用不受调用筛选限制，过滤变化会清理旧详情', async () => {
  mount();
  const user = userEvent.setup();
  const functions = screen.getByRole('combobox', {
    name: '同文件自由函数定义',
  });
  await user.selectOptions(functions, 'f0:context:210:250');
  expect(
    screen.getByRole('region', { name: '所选函数的同批引用' }),
  ).toHaveTextContent('本批没有词法关联引用，不表示该函数未被使用');
  await user.selectOptions(
    screen.getByRole('combobox', { name: '调用语法种类' }),
    'method',
  );
  expect(functions).toHaveValue('');
  await user.selectOptions(functions, 'f0:context:130:200');
  expect(
    within(screen.getByRole('list', { name: 'Rust 调用位置' })).getAllByRole(
      'listitem',
    ),
  ).toHaveLength(1);
  expect(
    screen.getByRole('region', { name: '所选函数的同批引用' }),
  ).toHaveTextContent('同批词法关联引用：3 处');
  expect(
    screen.getByText(/不受当前上下文或调用种类筛选限制/),
  ).toBeInTheDocument();
});

it.each(Object.entries(rustUnresolvedReasons))(
  '未解析原因%s按闭集中文显示且无猜测定义',
  async (reason, explanation) => {
    const data = rustAnalysis();
    Object.assign(data.links!.associations[2], { reason });
    mount(data);
    await userEvent.click(
      screen.getByRole('button', {
        name: '查看调用 missing 的词法关联，第 5 行字节列 5',
      }),
    );
    const details = screen.getByRole('region', { name: 'Rust 词法关联详情' });
    expect(details).toHaveTextContent(`未解析：${explanation}`);
    expect(
      within(details).queryByRole('region', { name: '所选函数的同批引用' }),
    ).not.toBeInTheDocument();
  },
);

it('旧服务缺links保留T044位置与说明；名称仍为纯文本', () => {
  const data = rustData();
  data.graph!.calls[0].name = '<img src=x onerror=alert(1)>';
  mount(data);
  expect(
    screen.getByText('当前服务未提供词法关联；保留调用位置，目标未解析。'),
  ).toBeInTheDocument();
  expect(screen.getByRole('list', { name: 'Rust 调用位置' })).toHaveTextContent(
    '<img src=x onerror=alert(1)>',
  );
  expect(
    screen.queryByRole('combobox', { name: '同文件自由函数定义' }),
  ).not.toBeInTheDocument();
  expect(document.querySelector('img, script')).toBeNull();
});

it('批标识明确是内容身份，不声称浏览器重算哈希或全仓库完整性', () => {
  mount();
  expect(screen.getByText('a'.repeat(64))).toBeInTheDocument();
  expect(
    screen.getByText(/前端没有源码正文，无法独立重算内容哈希/),
  ).toHaveTextContent('不是文件系统快照或新鲜度证明');
});
