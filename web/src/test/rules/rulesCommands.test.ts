import { beforeEach, expect, it, vi } from 'vitest';
import { arrangeCommands } from '../../commands/arrange';
import { buildCommands, executeCommand } from '../../commands/registry';
import { paletteContext } from '../commands/paletteFixtures';

const fetchMock = vi.fn<typeof fetch>();
beforeEach(() => {
  fetchMock.mockReset();
  vi.stubGlobal('fetch', fetchMock);
});

function command(id: string) {
  const found = buildCommands(paletteContext().context).find(
    (entry) => entry.id === id,
  );
  if (!found) throw new Error(`缺少命令 ${id}`);
  return found;
}

it('⌘K「规则」只换视图；「新建规则」只打开空白的编辑面板：不提交、不发布、零请求', () => {
  const view = paletteContext();
  executeCommand(command('view:rules'), view.context);
  expect(view.actions.goTo.mock.calls).toEqual([[{ view: 'rules' }]]);

  const create = paletteContext();
  const entry = command('action:rule-new');
  expect(entry.group).toBe('action');
  expect(entry.irreversible).toBeUndefined();
  executeCommand(entry, create.context);
  expect(create.actions.goTo.mock.calls).toEqual([
    [{ view: 'rules', panel: 'edit' }],
  ]);
  expect(create.actions.focusTarget).not.toHaveBeenCalled();
  expect(fetchMock).not.toHaveBeenCalled();
});

it('输入「规则」：动作组「新建规则」排第一，跳转组「规则」排在只以别名命中的「策略」前面', () => {
  const sections = arrangeCommands(
    buildCommands(paletteContext().context),
    '规则',
  );
  const labels = Object.fromEntries(
    sections.map((section) => [
      section.group,
      section.entries.map((entry) => entry.command.label),
    ]),
  );
  expect(labels.action[0]).toBe('新建规则');
  expect(labels.navigate).toEqual(['规则', '策略']);
});
