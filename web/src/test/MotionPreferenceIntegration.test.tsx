import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { MotionConfig, visualElementStore } from 'motion/react';
import { expect, it, vi } from 'vitest';
import { AppShell } from '../components/AppShell';
import { Documents } from '../components/Documents';
import { mediaPreference } from './dialogSurfaceHelpers';
import { jsonResponse } from './fixtures';
import { treeEntry, treePage } from './treeFixtures';

it('真实 MotionConfig 缓存随偏好刷新，保留焦点、输入与展开目录', async () => {
  const preference = mediaPreference(true);
  const fetchMock = vi.fn<typeof fetch>(async (input) => {
    const url = new URL(String(input), 'http://localhost');
    if (url.pathname === '/api/search')
      return jsonResponse({ generation: 7, hits: [], truncated: false });
    const path = url.searchParams.get('path') ?? '';
    return jsonResponse(
      treePage(
        path,
        path
          ? [treeEntry('资料/笔记.md', 'markdown')]
          : [treeEntry('资料', 'directory')],
      ),
    );
  });
  vi.stubGlobal('fetch', fetchMock);
  const { container } = render(
    <MotionConfig reducedMotion="user">
      <AppShell
        view="documents"
        onNavigate={vi.fn()}
        sourceLabel="测试库"
        projects={[]}
        onOpen={vi.fn()}
        actionCount={0}
        documentCount={1}
        toolbar={null}
      >
        <Documents
          projects={[]}
          generation={7}
          onOpen={vi.fn()}
          onRefresh={vi.fn()}
        />
      </AppShell>
    </MotionConfig>,
  );
  await screen.findByText('当前快照没有可读取的文档。');

  const searchButton = screen.getByRole('button', { name: '搜索' });
  const searchInput = screen.getByRole('searchbox');
  searchButton.focus();
  for (const reduced of [false, true, false]) {
    const previous = [
      container.querySelector('.navigation-highlight')!,
      container.querySelector('.document-mode-selected')!,
    ];
    preference.change(reduced);
    const current = [
      container.querySelector('.navigation-highlight')!,
      container.querySelector('.document-mode-selected')!,
    ];
    current.forEach((element, index) => {
      expect(element).not.toBe(previous[index]);
      // 验证 Motion 实际用来禁用动画的缓存，不只检查 React 的 layoutId 属性。
      const visual = visualElementStore.get(element)!;
      expect(visual.shouldReduceMotion).toBe(reduced);
      if (reduced) expect(visual.projection?.options.layoutId).toBeUndefined();
      else expect(visual.projection?.options.layoutId).toBeTruthy();
    });
    expect(searchButton).toHaveFocus();
    expect(screen.getByRole('searchbox')).toBe(searchInput);
  }

  fireEvent.click(screen.getByRole('button', { name: '文件树' }));
  const directory = await screen.findByRole('button', {
    name: '展开目录 资料',
  });
  preference.change(true);
  fireEvent.click(directory);
  await screen.findByRole('button', { name: '笔记.md，查看原文' });
  const children = container.querySelector<HTMLElement>('.tree-children')!;
  expect(children.style.transform).toBe('none');
  const loadedRequests = fetchMock.mock.calls.length;

  directory.focus();
  preference.change(false);
  expect(directory).toHaveFocus();
  expect(directory).toHaveAttribute('aria-expanded', 'true');
  expect(container.querySelector('.tree-children')).toBe(children);
  expect(fetchMock).toHaveBeenCalledTimes(loadedRequests);

  fireEvent.click(directory);
  fireEvent.click(directory);
  const animated = container.querySelector<HTMLElement>('.tree-children')!;
  expect(animated).not.toBe(children);
  expect(animated.style.transform).toBe('translateY(-3px)');
  expect(visualElementStore.get(animated)?.shouldReduceMotion).toBe(false);
  preference.change(true);
  await waitFor(() => expect(animated.style.transform).toBe('none'));
  expect(container.querySelector('.tree-children')).toBe(animated);
  expect(directory).toHaveFocus();
});
