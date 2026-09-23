import { useState } from 'react';
import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { expect, it, vi } from 'vitest';
import { AttachmentInfo } from '../components/AttachmentInfo';
import { CommandPalette } from '../components/CommandPalette';
import type { CommandPaletteContext } from '../components/CommandPalette';
import { useCommandPalette } from '../commands/useCommandPalette';
import { defaultViewLocation } from '../viewLocation';
import { jsonResponse } from './fixtures';
import { treeEntry } from './treeFixtures';

const context: CommandPaletteContext = {
  location: defaultViewLocation,
  projects: [],
  draftCount: 0,
  fileOperations: null,
  goTo: () => {},
  refreshSnapshot: () => {},
  openSource: () => {},
  openDrafts: () => {},
  openRecentlyDeleted: () => {},
  openStatus: () => {},
  focusTarget: () => {},
};

function ModalHarness() {
  const [attachment, setAttachment] = useState(false);
  const palette = useCommandPalette(true);
  return (
    <>
      <button onClick={() => setAttachment(true)}>查看附件</button>
      <button onClick={palette.show}>打开动作面板</button>
      {attachment ? (
        <AttachmentInfo
          entry={treeEntry('资料.pdf')}
          onClose={() => setAttachment(false)}
        />
      ) : null}
      {palette.open ? (
        <CommandPalette
          context={context}
          generation={7}
          onClose={palette.close}
          onRefresh={() => {}}
        />
      ) : null}
    </>
  );
}

it('附件模态期间不叠加动作面板，关闭后快捷键恢复且可再次切换关闭', async () => {
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
  fireEvent.click(screen.getByRole('button', { name: '打开动作面板' }));
  expect(screen.getAllByRole('dialog')).toEqual([attachment]);
  expect(fetchMock).not.toHaveBeenCalled();
  await user.click(screen.getByRole('button', { name: '关闭附件信息' }));
  expect(screen.getByRole('button', { name: '查看附件' })).toHaveFocus();
  fireEvent.keyDown(window, { key: 'k', metaKey: true });
  expect(screen.getByRole('dialog', { name: '动作面板' })).toBeInTheDocument();
  fireEvent.keyDown(window, { key: 'k', ctrlKey: true });
  expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
});
