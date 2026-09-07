import { useState } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { AttachmentInfo } from '../components/AttachmentInfo';
import { QuickSwitcher } from '../components/QuickSwitcher';
import { useQuickSwitcher } from '../useQuickSwitcher';
import { jsonResponse } from './fixtures';
import { treeEntry } from './treeFixtures';

function ModalHarness() {
  const [attachment, setAttachment] = useState(false);
  const quick = useQuickSwitcher(true);
  return (
    <>
      <button onClick={() => setAttachment(true)}>查看附件</button>
      <button onClick={quick.show}>打开快速切换</button>
      {attachment ? (
        <AttachmentInfo
          entry={treeEntry('资料.pdf')}
          onClose={() => setAttachment(false)}
        />
      ) : null}
      {quick.open ? (
        <QuickSwitcher
          projects={[]}
          generation={7}
          onOpen={() => {}}
          onClose={quick.close}
          onRefresh={() => {}}
        />
      ) : null}
    </>
  );
}

it('附件模态期间不叠加快速切换，关闭后快捷键恢复且可再次切换关闭', async () => {
  const fetchMock = vi
    .fn<typeof fetch>()
    .mockResolvedValue(
      jsonResponse({ generation: 7, hits: [], truncated: false }),
    );
  vi.stubGlobal('fetch', fetchMock);
  const user = userEvent.setup();
  render(<ModalHarness />);
  await user.click(screen.getByRole('button', { name: '查看附件' }));
  const attachment = screen.getByRole('dialog', { name: '附件信息' });
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
  // jsdom 没有原生 inert，额外验证按钮入口也不会叠加模态。
  fireEvent.click(screen.getByRole('button', { name: '打开快速切换' }));
  expect(screen.getAllByRole('dialog')).toEqual([attachment]);
  expect(fetchMock).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: '关闭附件信息' }));
  expect(screen.getByRole('button', { name: '查看附件' })).toHaveFocus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(screen.getByRole('dialog', { name: '快速切换' })).toBeInTheDocument();
  fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
