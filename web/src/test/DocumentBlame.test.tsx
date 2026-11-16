import { webcrypto } from 'node:crypto';
import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { beforeEach, expect, it, vi } from 'vitest';
import { DocumentDialog } from '../components/DocumentDialog';
import { sha256Hex } from '../blameModel';
import { ApiError, installSessionReader } from '../sessionHttp';
import { emptyNavigation } from './fixtures';

const CONTENT = '第一行\n第二行\n第三行\n';
const hex = (c: string) => c.repeat(64);
beforeEach(() => {
  if (!globalThis.crypto?.subtle) vi.stubGlobal('crypto', webcrypto);
});

async function blameBody(digest?: string) {
  return {
    vault_uuid: '00000000-0000-4000-8000-000000000001',
    path: 'a.md',
    revision: hex('a'),
    content_sha256: digest ?? (await sha256Hex(CONTENT)),
    lines: 3,
    versions: 1,
    more_history: true,
    runs: [
      {
        from_line: 1,
        to_line: 2,
        origin: 'version',
        history_id: hex('c'),
        revision: hex('d'),
        author: { kind: 'human', issuer: 'https://issuer', subject: '主人' },
        authored_at_ms: null,
      },
      {
        from_line: 3,
        to_line: 3,
        origin: 'beyond_history',
        history_id: null,
        revision: null,
        author: null,
        authored_at_ms: null,
      },
    ],
  };
}

function open() {
  render(
    <DocumentDialog
      source={{ path: 'a.md', line: 1 }}
      generation={1}
      onClose={vi.fn()}
      onRefresh={vi.fn()}
    />,
  );
}

const DOCUMENT = {
  path: 'a.md',
  title: '甲',
  content: CONTENT,
  navigation: emptyNavigation,
};
/** 原文和归属走同一个会话读取入口：按路径分开答，`blame` 只在问归属时被调。 */
function install(blame: () => Promise<unknown>) {
  return installSessionReader(async (url) =>
    url.startsWith('/api/v1/history/blame') ? blame() : DOCUMENT,
  );
}

it('逐行归属默认关：不点不读；点开后每段第一行写缩写，再点收起', async () => {
  const body = await blameBody();
  const asked: string[] = [];
  const uninstall = installSessionReader(async (url) => {
    if (!url.startsWith('/api/v1/history/blame')) return DOCUMENT;
    asked.push(url);
    return body;
  });
  try {
    const user = userEvent.setup();
    open();
    await screen.findByText('第一行');
    expect(asked).toEqual([]);
    expect(screen.queryByTitle(/第 1–2 行/)).not.toBeInTheDocument();

    await user.click(screen.getByText('显示每行是谁改的'));
    const first = await screen.findByLabelText(/第 1–2 行：人 主人/);
    expect(first).toHaveTextContent('人');
    expect(screen.getByLabelText(/第 3 行：早于可查历史/)).toHaveTextContent(
      '早',
    );
    // 同一段的第二行只留线，不重复写字。
    expect(screen.getAllByTitle(/第 1–2 行：人 主人/)).toHaveLength(2);
    expect(screen.getByText(/只回看最近 50 版/)).toBeInTheDocument();
    expect(asked).toEqual(['/api/v1/history/blame?path=a.md']);

    await user.click(screen.getByText('隐藏每行是谁改的'));
    expect(screen.queryByTitle(/第 1–2 行/)).not.toBeInTheDocument();
  } finally {
    uninstall();
  }
});

it('快照和当前正文不是同一版就不画，说明原因', async () => {
  const body = await blameBody(hex('0'));
  const uninstall = install(async () => body);
  try {
    const user = userEvent.setup();
    open();
    await screen.findByText('第一行');
    await user.click(screen.getByText('显示每行是谁改的'));
    expect(
      await screen.findByText(/不是同一版，行号对不上/),
    ).toBeInTheDocument();
    expect(screen.queryByTitle(/第 1–2 行/)).not.toBeInTheDocument();
  } finally {
    uninstall();
  }
});

it('服务不提供时说清楚，别的错误可以重试', async () => {
  let gone = true;
  const body = await blameBody();
  const uninstall = install(async () => {
    if (gone) throw new ApiError('没有这个接口', 404);
    return body;
  });
  try {
    const user = userEvent.setup();
    open();
    await screen.findByText('第一行');
    await user.click(screen.getByText('显示每行是谁改的'));
    expect(
      await screen.findByText(/当前服务不提供逐行归属/),
    ).toBeInTheDocument();
    gone = false;
    await user.click(screen.getByText('重试'));
    await waitFor(() =>
      expect(screen.getByLabelText(/第 1–2 行：人 主人/)).toBeInTheDocument(),
    );
  } finally {
    uninstall();
  }
});
