import { useCallback, useEffect, useRef, useState } from 'react';
import {
  isAbsent,
  readDesired,
  readPolicyStatus,
  readVersion,
} from '../policyApi';
import type {
  ListedDesired,
  PolicyStatus,
  VersionRecord,
} from '../policyModel';

export interface RulesSnapshot {
  status: PolicyStatus;
  desired: ListedDesired[];
  /** 版本号 → 版本记录；只取生效中的规则与能力集。 */
  records: ReadonlyMap<number, VersionRecord>;
}

export type RulesData =
  | { kind: 'loading' }
  | { kind: 'absent' }
  | { kind: 'error'; message: string }
  | { kind: 'ready'; snapshot: RulesSnapshot };

/**
 * 一次读齐规则视图要的全部事实：生效条目与草案并行读，再按条目的版本号并行取正文
 * （认证闸门在 sessionHttp：在途最多两个，其余排队）。版本记录与条目对不上（版本、类别、对象、摘要）
 * 就整份作废：部分或不一致的响应绝不当成空数据显示。
 */
export async function readRules(signal: AbortSignal): Promise<RulesSnapshot> {
  const [status, desired] = await Promise.all([
    readPolicyStatus(signal),
    readDesired(signal),
  ]);
  const wanted = status.entries.filter(
    (entry) => entry.kind === 'rules' || entry.kind === 'capability_set',
  );
  const fetched = await Promise.all(
    wanted.map((entry) => readVersion(entry.version, signal)),
  );
  const records = new Map<number, VersionRecord>();
  wanted.forEach((entry, index) => {
    const record = fetched[index];
    const { meta } = record;
    if (
      meta.version !== entry.version ||
      meta.kind !== entry.kind ||
      meta.subject !== entry.subject ||
      meta.body_sha256 !== entry.body_sha256
    )
      throw new Error('规则正文与生效条目对不上，请重新读取。');
    records.set(entry.version, record);
  });
  return { status, desired, records };
}

/**
 * 规则视图的数据：挂载时读一次；动作之后 `reload` 重读，读完才换画面（不闪回读取中），
 * 失败抛给发起动作的一方在原处说明。未接入策略平面（404 policy_plane_absent）单列一态；
 * 读取失败只在人点「重新读取」时再读，绝不自动重试。
 */
export function useRulesData() {
  const [data, setData] = useState<RulesData>({ kind: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const lifetime = useRef<AbortController | null>(null);

  useEffect(() => {
    const controller = new AbortController();
    lifetime.current = controller;
    readRules(controller.signal).then(
      (snapshot) => {
        if (!controller.signal.aborted) setData({ kind: 'ready', snapshot });
      },
      (error: unknown) => {
        if (controller.signal.aborted) return;
        setData(
          isAbsent(error)
            ? { kind: 'absent' }
            : {
                kind: 'error',
                message:
                  error instanceof Error ? error.message : '读取规则失败。',
              },
        );
      },
    );
    return () => controller.abort();
  }, [attempt]);

  const reload = useCallback(async () => {
    const controller = lifetime.current;
    if (!controller || controller.signal.aborted) return;
    const snapshot = await readRules(controller.signal);
    if (!controller.signal.aborted) setData({ kind: 'ready', snapshot });
  }, []);

  const retry = useCallback(() => {
    setData({ kind: 'loading' });
    setAttempt((count) => count + 1);
  }, []);

  return { data, reload, retry };
}
