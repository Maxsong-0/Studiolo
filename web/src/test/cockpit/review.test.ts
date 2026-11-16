import { describe, expect, it } from 'vitest';
import {
  REVIEW_ORIGIN,
  VERDICT_TEXT,
  approveBlocker,
  publishKey,
  reviewLabel,
  reviewOrigin,
  reviewSignature,
  reviewVerdict,
  validatedNotice,
  withdrawKey,
  withdrawnNotice,
} from '../../cockpit/review';
import { policyEntryKey } from '../../cockpit/pending';
import { desiredTitle } from '../../cockpit/queueText';
import type { RecentAuth } from '../../policy/recentAuth';
import { desiredList, policyStatus } from '../../policyModel';
import type { ListedDesired } from '../../policyModel';
import {
  DESIRED_ID,
  HASH,
  desiredBody,
  listBody,
  reportBody,
  statusBody,
} from '../policy/policyFixtures';

type Body = Record<string, unknown>;

const MOVED = 'c'.repeat(64);
const NEWER = '01ABCDEFGHJKMNPQRSTVWXYZ09';

/** 一份草案（经真实解码器）；`blocked` 是同一提交者更新的、挡住它的草案号。 */
function draft(overrides: Body = {}, blocked: string[] = []): ListedDesired {
  const rows = [desiredBody(overrides)];
  if (blocked.length > 0)
    rows.unshift(
      ...blocked.map((id) =>
        desiredBody({ id, submitted_at: '2026-09-18T00:00:00Z' }),
      ),
    );
  const table = { [DESIRED_ID]: blocked };
  const [found] = desiredList(listBody(rows, table, table)).filter(
    (entry) => entry.id === DESIRED_ID,
  );
  return found;
}

const status = (overrides: Body = {}) => policyStatus(statusBody(overrides));

const VALID: RecentAuth = {
  kind: 'valid',
  window: { remainingMs: 60_000, at: 0 },
};

describe('就地审批的名字和键', () => {
  it('反馈的落点、两个执行预览的键、对象的名字', () => {
    expect(REVIEW_ORIGIN).toBe('review:');
    expect(reviewOrigin(DESIRED_ID)).toBe(`review:${DESIRED_ID}`);
    // 发布预览的键与策略页同一写法：同一份草案在两处开的是同一个预览。
    expect(publishKey(DESIRED_ID)).toBe(`publish:${DESIRED_ID}`);
    expect(withdrawKey(DESIRED_ID)).toBe(`withdraw:${DESIRED_ID}`);
    expect(reviewLabel(draft())).toBe('能力集 claude');
    expect(reviewLabel(draft({ kind: 'rules', subject: 'CLAUDE.md' }))).toBe(
      '规则母本 CLAUDE.md',
    );
  });

  it('名字和队列里那一行的标题是同一个：下线申请带「下线」', () => {
    const retire = draft({
      body: null,
      report: reportBody({ retire: true }),
    });
    expect(reviewLabel(retire)).toBe('下线能力集 claude');
    expect(reviewLabel(retire)).toBe(desiredTitle(retire));
    expect(withdrawnNotice(retire)).toBe(
      `已撤回 下线能力集 claude（草案 ${DESIRED_ID.slice(0, 12)}）。`,
    );
  });

  it('反馈的落点去掉前缀就是草案号，队列据此找回是哪一行', () => {
    const origin = reviewOrigin(DESIRED_ID);
    expect(policyEntryKey(origin.slice(REVIEW_ORIGIN.length))).toBe(
      `policy:${DESIRED_ID}`,
    );
  });
});

describe('不用核对就知道批不了的原因', () => {
  it('校验通过、没被更新的草案挡住：能批', () => {
    expect(approveBlocker(draft())).toBeNull();
    expect(
      approveBlocker(draft({ status: 'pending', submitter_kind: 'agent' })),
    ).toBeNull();
  });

  it('还没有校验报告：先校验', () => {
    expect(
      approveBlocker(
        draft({ status: 'pending', report: null, validated_at: null }),
      ),
    ).toBe('这份草案还没有校验报告：先校验。');
  });

  it('校验没通过：去策略页看问题，或者在这里退回', () => {
    expect(
      approveBlocker(
        draft({ status: 'pending', report: reportBody({ ok: false }) }),
      ),
    ).toBe('校验没通过：去策略页看逐条问题，改好重交，或者在这里退回。');
  });

  it('同一提交者还有更新的未发布草案：只能发最新的那一份', () => {
    const reason = approveBlocker(draft({}, [NEWER]));
    expect(reason).toContain('同一提交者还有更新的未发布草案');
    expect(reason).toContain(NEWER.slice(0, 12));
  });
});

describe('核对之后的结论', () => {
  it('报告是照现在生效的内容算的、入口有验证码路径：可以展开预览', () => {
    const others: RecentAuth[] = [
      VALID,
      { kind: 'none' },
      // 没读成近期认证不拦着：要不要认证到发布那一刻由服务端说。
      { kind: 'unknown' },
    ];
    for (const auth of others)
      expect(reviewVerdict(draft(), status(), auth)).toEqual({
        kind: 'ready',
      });
  });

  it('校验之后生效内容变过（基准摘要对不上现在的）：报告旧了', () => {
    expect(
      reviewVerdict(draft(), status({ effective_hash: MOVED }), VALID),
    ).toEqual({ kind: 'stale' });
    expect(
      reviewVerdict(
        draft({ report: reportBody({ base_effective_hash: MOVED }) }),
        status(),
        VALID,
      ),
    ).toEqual({ kind: 'stale' });
  });

  it('没有校验报告也算对不上（不会走到预览）', () => {
    expect(
      reviewVerdict(
        draft({ status: 'pending', report: null, validated_at: null }),
        status(),
        VALID,
      ),
    ).toEqual({ kind: 'stale' });
  });

  it('本入口没有验证码路径（策略状态这么说，或者因子状态这么说）：发布不了', () => {
    expect(
      reviewVerdict(draft(), status({ l3_available: false }), VALID),
    ).toEqual({ kind: 'no-path' });
    expect(reviewVerdict(draft(), status(), { kind: 'no-path' })).toEqual({
      kind: 'no-path',
    });
  });

  it('本页没有受控会话：没法就地认证', () => {
    expect(reviewVerdict(draft(), status(), { kind: 'no-session' })).toEqual({
      kind: 'no-session',
    });
  });

  it('先说发布不了，再说报告旧了：重新校验也救不了没有路径的入口', () => {
    const moved = status({ effective_hash: MOVED });
    expect(
      reviewVerdict(
        draft(),
        status({ l3_available: false, effective_hash: MOVED }),
        VALID,
      ),
    ).toEqual({ kind: 'no-path' });
    expect(reviewVerdict(draft(), moved, { kind: 'no-path' })).toEqual({
      kind: 'no-path',
    });
    expect(reviewVerdict(draft(), moved, { kind: 'no-session' })).toEqual({
      kind: 'no-session',
    });
  });

  it('每种没过的结论都有一句说明', () => {
    expect(Object.keys(VERDICT_TEXT).sort()).toEqual([
      'no-path',
      'no-session',
      'stale',
    ]);
    expect(VERDICT_TEXT.stale).toContain('先重新校验');
    expect(VERDICT_TEXT['no-path']).toContain('只能查看和退回');
    expect(VERDICT_TEXT['no-session']).toContain('重新登录');
  });
});

describe('草案的指纹', () => {
  const base = reviewSignature(draft());

  it('列表重读一遍、内容没变：指纹不变', () => {
    expect(reviewSignature(draft())).toBe(base);
  });

  it('状态、校验时刻、校验基准、正文摘要任何一样变了：指纹跟着变', () => {
    const changed = [
      draft({ status: 'pending' }),
      draft({ validated_at: '2026-09-18T00:00:00Z' }),
      draft({ report: reportBody({ base_effective_hash: MOVED }) }),
      draft({ report: reportBody({ body_sha256: MOVED }) }),
      draft({ status: 'pending', report: null, validated_at: null }),
    ].map(reviewSignature);
    for (const signature of changed) expect(signature).not.toBe(base);
    expect(new Set(changed).size).toBe(changed.length);
  });

  it('与指纹无关的字段（提交时刻）变了：不变', () => {
    expect(
      reviewSignature(draft({ submitted_at: '2026-09-16T00:00:00Z' })),
    ).toBe(base);
  });

  it('基准摘要取自报告', () => {
    expect(base).toContain(HASH);
  });
});

describe('做完之后说的那一句：点明是哪一份', () => {
  const which = `能力集 claude（草案 ${DESIRED_ID.slice(0, 12)}）`;

  it('AI 的提案是「退回」，自己交的是「撤回」', () => {
    expect(
      withdrawnNotice(
        draft({
          status: 'pending',
          submitter_kind: 'agent',
          agent_type: 'claude',
        }),
      ),
    ).toBe(`已退回 ${which}。`);
    expect(withdrawnNotice(draft())).toBe(`已撤回 ${which}。`);
  });

  it('校验通过：写新的状态', () => {
    expect(validatedNotice(draft(), 'validated')).toBe(
      `已重新校验 ${which}：已校验，可发布。`,
    );
  });

  it('校验没通过：它会离开队列，写明去了哪里', () => {
    expect(validatedNotice(draft(), 'rejected')).toBe(
      `已重新校验 ${which}：校验未通过，它不再排在这里等你点头。去策略页的待处理草案里看逐条问题，改好重交。`,
    );
  });
});
