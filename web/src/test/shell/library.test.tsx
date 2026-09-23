import { render, screen, within } from '@testing-library/react';
import { expect, it, vi } from 'vitest';
import { App } from '../../App';
import { AppShell } from '../../components/AppShell';
import { defaultApiResponse } from '../fixtures';

function shell(sourceLabel?: string | null) {
  return render(
    <AppShell
      view="cockpit"
      onNavigate={vi.fn()}
      sourceLabel={sourceLabel}
      actionCount={3}
      documentCount={12}
    >
      <p>正文</p>
    </AppShell>,
  );
}

it('顶栏字标右侧显示当前库名，title 给全名；超长只在视觉上省略，读屏读到全名', () => {
  const name = '某个很长很长的只读副本库名 · 超过二十个字符也照样给出全名';
  shell(name);
  const banner = screen.getByRole('banner');
  const library = within(banner).getByTitle(name);
  expect(library).toHaveClass('shell-library');
  expect(within(library).getByText(name)).toBeInTheDocument();
  expect(library).toHaveTextContent(`当前库：${name}`);
  // 紧跟字标，排在主导航之前（规范第 5 节：库名在字标右侧）
  expect(banner.querySelector('.shell-brand')?.nextElementSibling).toBe(
    library,
  );
  expect(
    library.compareDocumentPosition(
      within(banner).getByRole('navigation', { name: '主导航' }),
    ) & Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
});

it('还没读到快照（null 或不传）时显示「—」，title 与读屏都写明原因', () => {
  const { unmount } = shell(null);
  const pending = within(screen.getByRole('banner')).getByTitle(
    '尚未读到工作台快照',
  );
  expect(pending).toHaveClass('shell-library');
  expect(pending).toHaveTextContent('—');
  expect(pending).toHaveTextContent('当前库：尚未读到工作台快照');
  unmount();

  shell();
  expect(
    within(screen.getByRole('banner')).getByTitle('尚未读到工作台快照'),
  ).toHaveTextContent('—');
});

it('快照给出空库名时也显示「—」，原因写成快照没有给出库名，不冒充「尚未读到」', () => {
  shell('   ');
  const empty = within(screen.getByRole('banner')).getByTitle(
    '工作台快照没有给出库名',
  );
  expect(empty).toHaveTextContent('—');
  expect(
    within(screen.getByRole('banner')).queryByTitle('尚未读到工作台快照'),
  ).not.toBeInTheDocument();
});

it('App 把工作台快照的 source_label 交给顶栏：读到之前「—」，读到之后是库名', async () => {
  vi.stubGlobal(
    'fetch',
    vi.fn<typeof fetch>(async (input) => defaultApiResponse(input)),
  );
  render(<App />);
  const banner = screen.getByRole('banner');
  expect(within(banner).getByTitle('尚未读到工作台快照')).toHaveTextContent(
    '—',
  );
  const library = await within(banner).findByTitle('测试只读库');
  expect(library).toHaveTextContent('当前库：测试只读库');
  expect(
    within(banner).queryByTitle('尚未读到工作台快照'),
  ).not.toBeInTheDocument();
});
