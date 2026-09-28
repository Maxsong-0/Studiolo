import { screen, waitFor, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';
import { versionRecord } from '../../policyModel';
import { displayTime } from '../../rules/labels';
import { formatViewLocation, parseViewLocation } from '../../viewLocation';
import { mountRules, serveFixtures } from './rulesHarness';

// 源码编辑器换成受控的多行文本框：jsdom 里不跑 CodeMirror，受控接口（value／onChange）与真组件一致。
vi.mock('../../components/MarkdownEditor', () => ({
  MarkdownEditor: ({
    value,
    onChange,
  }: {
    value: string;
    onChange: (value: string) => void;
  }) => (
    <textarea
      aria-label="Markdown 源码"
      value={value}
      onChange={(event) => onChange(event.target.value)}
    />
  ),
}));

const CODEX_CLAUDE_MD = 'codex 读取 CLAUDE.md';

describe('规则视图', { timeout: 60_000 }, () => {
  it('列表：每条规则一行——版本、生效时刻、读者（名单里列了它的类型；缺字段的 codex 不算）、未发布草案', async () => {
    const server = serveFixtures();
    await mountRules('#view=rules');
    const table = await screen.findByRole('table', { name: '规则一览' });
    const rows = within(table).getAllByRole('row').slice(1);
    const status = server.read('/api/v1/policy') as {
      entries: { subject: string; activated_at: string }[];
    };
    const at = (subject: string) =>
      displayTime(
        status.entries.find((entry) => entry.subject === subject)!.activated_at,
      );
    expect(
      rows.map((row) => [
        within(row).getByRole('rowheader').textContent,
        ...within(row)
          .getAllByRole('cell')
          .map((cell) => cell.textContent),
      ]),
    ).toEqual([
      ['CLAUDE.md', 'v1', at('CLAUDE.md'), 'claude', '—'],
      ['通用工作规则.md', 'v2', at('通用工作规则.md'), 'claude', '—'],
    ]);
    expect(screen.getByRole('tab', { name: '列表' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
  });

  it('矩阵：切「codex 读 CLAUDE.md」＝提交 codex 的能力集草案，正文只多出 rules_whitelist，其余逐字不变', async () => {
    const server = serveFixtures();
    const user = await mountRules('#view=rules&panel=access');
    const cell = await screen.findByRole('switch', { name: CODEX_CLAUDE_MD });
    expect(cell).toHaveAttribute('aria-checked', 'false');
    expect(
      screen.getByRole('switch', { name: 'claude 读取 CLAUDE.md' }),
    ).toHaveAttribute('aria-checked', 'true');
    expect(
      screen.getByRole('columnheader', { name: /^codex/ }),
    ).toHaveTextContent('未设（零规则）');
    const effective = versionRecord(
      server.read('/api/v1/policy/versions/4'),
    ).body!;

    await user.click(cell);

    await waitFor(() =>
      expect(server.posts('/api/v1/policy/desired')).toHaveLength(1),
    );
    expect(server.posts('/api/v1/policy/desired')[0].body).toEqual({
      kind: 'capability_set',
      subject: 'codex',
      body: effective.replace(
        '"deny_by_default":true}',
        '"deny_by_default":true,"rules_whitelist":["CLAUDE.md"]}',
      ),
    });
    // 草案就位：这一格按草案显示为可读并标「待发布」，这一列仍能继续改（T-160g）；下方给出校验报告与发布。
    const pending = await screen.findByRole('region', { name: '能力集 codex' });
    const after = screen.getByRole('switch', { name: CODEX_CLAUDE_MD });
    expect(after).toHaveAttribute('aria-checked', 'true');
    expect(after).toHaveAttribute('data-pending', 'true');
    expect(after).toHaveAccessibleDescription(/待发布/);
    expect(after).toBeEnabled();
    expect(
      within(pending).getByRole('group', { name: '校验报告' }),
    ).toHaveTextContent('read.rules_whitelist');
    expect(server.posts('/api/v1/policy/activate')).toHaveLength(0);
  });

  it('矩阵：已有草案时再切一格＝在草案上追加（交新撤旧），一轮发布两条都在（T-160g）', async () => {
    const server = serveFixtures();
    const user = await mountRules('#view=rules&panel=access');
    await user.click(
      await screen.findByRole('switch', { name: CODEX_CLAUDE_MD }),
    );
    await waitFor(() =>
      expect(server.posts('/api/v1/policy/desired')).toHaveLength(1),
    );
    const first = (
      server.read('/api/v1/policy/desired') as {
        desired: {
          id: string;
          subject: string;
          status: string;
          submitter_kind: string;
        }[];
      }
    ).desired.find(
      (record) =>
        record.subject === 'codex' &&
        record.submitter_kind === 'owner' &&
        record.status !== 'withdrawn',
    )!;
    const second = await screen.findByRole('switch', {
      name: 'codex 读取 通用工作规则.md',
    });
    expect(second).toBeEnabled();

    await user.click(second);

    await waitFor(() =>
      expect(server.posts('/api/v1/policy/withdraw')).toHaveLength(1),
    );
    const posted = server.posts('/api/v1/policy/desired');
    expect(posted).toHaveLength(2);
    const amended = posted[1].body as { subject: string; body: string };
    expect(amended.subject).toBe('codex');
    expect(JSON.parse(amended.body).read.rules_whitelist).toEqual([
      'CLAUDE.md',
      '通用工作规则.md',
    ]);
    // 先交后撤：新草案的提交排在撤回旧草案之前。
    const order = server.sent
      .filter((request) => request.method === 'POST')
      .map((request) => request.path);
    expect(order.lastIndexOf('/api/v1/policy/desired')).toBeLessThan(
      order.lastIndexOf('/api/v1/policy/withdraw'),
    );
    expect(server.posts('/api/v1/policy/withdraw')[0].body).toEqual({
      desired_id: first.id,
    });
    await waitFor(() =>
      expect(
        screen.getByRole('switch', { name: 'codex 读取 通用工作规则.md' }),
      ).toHaveAttribute('aria-checked', 'true'),
    );
    expect(
      screen.getByRole('switch', { name: CODEX_CLAUDE_MD }),
    ).toHaveAttribute('aria-checked', 'true');
    expect(server.posts('/api/v1/policy/activate')).toHaveLength(0);
  });

  it('矩阵：收回「claude 读通用工作规则.md」＝名单原位去掉这一条，别的字段一字不动', async () => {
    const server = serveFixtures();
    const user = await mountRules('#view=rules&panel=access');
    const cell = await screen.findByRole('switch', {
      name: 'claude 读取 通用工作规则.md',
    });
    expect(cell).toHaveAttribute('aria-checked', 'true');
    const effective = versionRecord(
      server.read('/api/v1/policy/versions/3'),
    ).body!;

    await user.click(cell);

    await waitFor(() =>
      expect(server.posts('/api/v1/policy/desired')).toHaveLength(1),
    );
    expect(server.posts('/api/v1/policy/desired')[0].body).toEqual({
      kind: 'capability_set',
      subject: 'claude',
      body: effective.replace(
        '"rules_whitelist":["CLAUDE.md","通用工作规则.md"]',
        '"rules_whitelist":["CLAUDE.md"]',
      ),
    });
  });

  it('发布被二次认证挡下时就地弹窗，认证通过后原样重试同一次发布，矩阵随即按新的生效显示', async () => {
    const server = serveFixtures();
    const user = await mountRules('#view=rules&panel=access');
    await user.click(
      await screen.findByRole('switch', { name: CODEX_CLAUDE_MD }),
    );
    const pending = await screen.findByRole('region', { name: '能力集 codex' });
    await user.click(
      within(pending).getByRole('button', { name: '发布（需二次认证）' }),
    );
    const preview = screen.getByRole('group', { name: /^执行预览/ });
    expect(server.posts('/api/v1/policy/activate')).toHaveLength(0);
    await user.click(within(preview).getByRole('button', { name: '确认发布' }));

    const dialog = await screen.findByRole('dialog', {
      name: '二次认证后继续',
    });
    expect(server.posts('/api/v1/policy/activate')).toHaveLength(1);
    expect(
      within(dialog).getByText(/本次动作：发布 能力集 codex/),
    ).toBeInTheDocument();
    await user.type(await within(dialog).findByLabelText('验证码'), '123456');
    await user.click(
      within(dialog).getByRole('button', { name: '完成认证并继续' }),
    );

    expect(
      await screen.findByText(/^能力集 codex：已发布为 v5/),
    ).toBeInTheDocument();
    expect(screen.queryByRole('dialog')).not.toBeInTheDocument();
    const activations = server
      .posts('/api/v1/policy/activate')
      .map((request) => request.body);
    // 重试与首次逐字节相同：人审过哪一份生效中内容就按哪一份发布，验证码不混进策略请求。
    expect(activations).toHaveLength(2);
    expect(activations[1]).toEqual(activations[0]);
    expect(JSON.stringify(activations)).not.toContain('123456');
    expect(
      server.posts('/api/v1/auth/step-up').map((request) => request.body),
    ).toEqual([{ code: '123456' }]);
    await waitFor(() => {
      const cell = screen.getByRole('switch', { name: CODEX_CLAUDE_MD });
      expect(cell).toHaveAttribute('aria-checked', 'true');
      expect(cell).not.toHaveAttribute('data-pending');
      expect(cell).toBeEnabled();
    });
    expect(
      screen.queryByRole('region', { name: '能力集 codex' }),
    ).not.toBeInTheDocument();
  });

  it('位置：#view=rules&panel=edit&item=规则名 解析成规则的编辑面板，直接打开那条规则', async () => {
    const hash = `#view=rules&panel=edit&item=${encodeURIComponent('通用工作规则.md')}`;
    expect(parseViewLocation(hash)).toEqual({
      view: 'rules',
      project: null,
      panel: 'edit',
      item: '通用工作规则.md',
    });
    expect(formatViewLocation(parseViewLocation(hash))).toBe(hash.slice(1));
    const server = serveFixtures();
    await mountRules(hash);
    expect(
      await screen.findByRole('heading', { name: '编辑 通用工作规则.md' }),
    ).toBeInTheDocument();
    expect(screen.getByRole('tab', { name: '编辑' })).toHaveAttribute(
      'aria-selected',
      'true',
    );
    // 没有未发布草案：编辑区以生效正文为底。
    expect(await screen.findByLabelText('Markdown 源码')).toHaveValue(
      versionRecord(server.read('/api/v1/policy/versions/2')).body,
    );
  });

  it('新建：规则名按后端口径预校验；保存＝提交 rules 草案，成功后位置改成这条规则、下方出现校验报告', async () => {
    const server = serveFixtures();
    const user = await mountRules('#view=rules&panel=edit');
    await screen.findByRole('heading', { name: '新建规则' });
    const name = screen.getByLabelText('规则名');
    await user.type(name, 'docs/交接.md');
    expect(screen.getByRole('alert')).toHaveTextContent('不能含 /');
    await user.clear(name);
    await user.type(name, '交接规则.md');
    expect(screen.queryByRole('alert')).not.toBeInTheDocument();
    await user.type(
      await screen.findByLabelText('Markdown 源码'),
      '# 交接规则{Enter}{Enter}1. 交接文档必须写绝对日期。',
    );
    await user.click(
      screen.getByRole('button', { name: '保存为规则草案（不发布）' }),
    );

    await waitFor(() =>
      expect(server.posts('/api/v1/policy/desired')).toHaveLength(1),
    );
    expect(server.posts('/api/v1/policy/desired')[0].body).toEqual({
      kind: 'rules',
      subject: '交接规则.md',
      body: '# 交接规则\n\n1. 交接文档必须写绝对日期。',
    });
    expect(
      await screen.findByRole('heading', { name: '编辑 交接规则.md' }),
    ).toBeInTheDocument();
    expect(parseViewLocation(window.location.hash)).toEqual({
      view: 'rules',
      project: null,
      panel: 'edit',
      item: '交接规则.md',
    });
    expect(screen.getByRole('group', { name: '校验报告' })).toBeInTheDocument();
    expect(screen.getByText(/已提交规则草案并校验通过/)).toBeInTheDocument();
  });
});
