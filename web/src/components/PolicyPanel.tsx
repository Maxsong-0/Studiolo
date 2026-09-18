import { useCallback, useEffect, useRef, useState } from 'react';
import {
  activateFailure,
  activatePolicy,
  readDesired,
  readPolicyStatus,
  readVersion,
  readVersions,
  revokeAgent,
  submitDesired,
  validateDesired,
  withdrawDesired,
} from '../policyApi';
import type {
  ActivateRequest,
  DesiredInput,
  DesiredRecord,
  PolicyStatus,
  RevokeRequest,
  VersionMeta,
  VersionRecord,
} from '../policyModel';
import {
  KIND_LABEL,
  LEVEL_LABEL,
  STATUS_LABEL,
  shortHash,
} from '../policyModel';
import { DesiredForm, RevokeForm } from './PolicyForms';
import { PolicyReport } from './PolicyReport';
import { Icon } from './Icon';

type Loaded = {
  status: PolicyStatus;
  desired: DesiredRecord[];
  versions: VersionMeta[];
};

function when(value: string) {
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleString();
}

/** 策略平面视图：概览、desired 提交与校验、L3 发布与回滚、三级吊销。全部动作先校验后动手，永不自动发布。 */
export function PolicyPanel({ initial }: { initial: Loaded }) {
  const [data, setData] = useState<Loaded>(initial);
  const [busy, setBusy] = useState(false);
  const [notice, setNotice] = useState('');
  const [error, setError] = useState('');
  const [selected, setSelected] = useState<DesiredRecord | null>(null);
  const [version, setVersion] = useState<VersionRecord | null>(null);
  const lifetime = useRef(new AbortController());
  useEffect(() => {
    const controller = lifetime.current;
    return () => controller.abort();
  }, []);

  const reload = useCallback(async () => {
    const signal = lifetime.current.signal;
    const [status, desired, versions] = await Promise.all([
      readPolicyStatus(signal),
      readDesired(signal),
      readVersions(signal),
    ]);
    if (!signal.aborted) setData({ status, desired, versions });
  }, []);

  async function act(operation: () => Promise<string>) {
    if (busy) return;
    setBusy(true);
    setError('');
    setNotice('');
    try {
      const message = await operation();
      if (lifetime.current.signal.aborted) return;
      setNotice(message);
      await reload();
    } catch (cause) {
      if (lifetime.current.signal.aborted) return;
      setError(
        cause instanceof Error ? cause.message : '操作未完成，请重新核对。',
      );
      await reload().catch(() => undefined);
    } finally {
      if (!lifetime.current.signal.aborted) setBusy(false);
    }
  }

  function submit(input: DesiredInput) {
    void act(async () => {
      const record = await submitDesired(input, lifetime.current.signal);
      setSelected(record);
      return record.report?.ok
        ? `已提交并校验通过（${record.id}）；发布前请逐条确认扩大项。`
        : `已提交但校验未通过（${record.id}）；请按逐条问题修改后重新提交。`;
    });
  }
  function validate(record: DesiredRecord) {
    void act(async () => {
      const updated = await validateDesired(record.id, lifetime.current.signal);
      setSelected(updated);
      return `已重新校验：${STATUS_LABEL[updated.status]}。`;
    });
  }
  function withdraw(record: DesiredRecord) {
    void act(async () => {
      await withdrawDesired(record.id, lifetime.current.signal);
      setSelected(null);
      return '已撤回。';
    });
  }
  function activate(request: ActivateRequest, label: string) {
    void act(async () => {
      try {
        const outcome = await activatePolicy(request, lifetime.current.signal);
        setSelected(null);
        setVersion(null);
        if (outcome.kind === 'released')
          return `已解除全局吊销${outcome.audit_confirmed ? '' : '（审计未确认，请核对账本）'}。`;
        return `${label}：已发布为 v${outcome.activation.version}，effective ${shortHash(
          outcome.activation.effective_hash,
        )}${outcome.activation.cleared_type_revocation ? '，并解除了该类型的吊销' : ''}${
          outcome.audit_confirmed ? '' : '（审计未确认，请核对账本）'
        }。`;
      } catch (cause) {
        throw new Error(activateFailure(cause));
      }
    });
  }
  function revoke(request: RevokeRequest) {
    void act(async () => {
      const outcome = await revokeAgent(request, lifetime.current.signal);
      return outcome.created
        ? `已吊销 ${LEVEL_LABEL[outcome.level]} ${outcome.target}（即刻生效）。`
        : `${LEVEL_LABEL[outcome.level]} ${outcome.target} 已处于吊销中。`;
    });
  }
  function open(meta: VersionMeta) {
    void act(async () => {
      setVersion(await readVersion(meta.version, lifetime.current.signal));
      return `已读取 v${meta.version}。`;
    });
  }

  const { status, desired, versions } = data;
  const actionable = desired.filter((record) =>
    ['pending', 'validated', 'rejected'].includes(record.status),
  );
  return (
    <div className="policy-panel">
      {notice ? <p role="status">{notice}</p> : null}
      {error ? (
        <p className="policy-warning" role="alert">
          {error}
        </p>
      ) : null}
      <section className="policy-section" aria-labelledby="policy-overview">
        <h2 id="policy-overview">
          <Icon name="shield" />
          effective
        </h2>
        <p className="policy-summary">
          当前 effective{' '}
          <code title={status.effective_hash}>
            {shortHash(status.effective_hash)}
          </code>
          ；待处理 desired {status.pending_desired}；
          {status.l3_available
            ? '本入口具备 One-time Code 路径'
            : '本入口没有 One-time Code 路径，只能查看与吊销'}
          。
        </p>
        {status.global_revoked ? (
          <p className="policy-warning" role="alert">
            全局吊销生效中：所有 AI 都被拒绝。解除需要二次认证。
          </p>
        ) : null}
        <table className="policy-table" aria-label="现行各项">
          <thead>
            <tr>
              <th>类别</th>
              <th>subject</th>
              <th>版本</th>
              <th>sha256</th>
              <th>发布时间</th>
            </tr>
          </thead>
          <tbody>
            {status.entries.length ? (
              status.entries.map((entry) => (
                <tr key={`${entry.kind}:${entry.subject}`}>
                  <td>{KIND_LABEL[entry.kind]}</td>
                  <td>{entry.subject}</td>
                  <td>v{entry.version}</td>
                  <td>
                    <code title={entry.body_sha256}>
                      {shortHash(entry.body_sha256)}
                    </code>
                  </td>
                  <td>{when(entry.activated_at)}</td>
                </tr>
              ))
            ) : (
              <tr>
                <td colSpan={5}>
                  尚无任何生效母本：所有 AI 类型都是零权限，直到发布其能力集。
                </td>
              </tr>
            )}
          </tbody>
        </table>
        {status.revocations.length ? (
          <ul className="policy-revocations" aria-label="生效中的吊销">
            {status.revocations.map((item) => (
              <li key={item.seq}>
                {LEVEL_LABEL[item.level]} <code>{item.target}</code>：
                {item.reason}（{when(item.revoked_at)}）
              </li>
            ))}
          </ul>
        ) : null}
        {status.global_revoked ? (
          <button
            className="outline-button"
            disabled={busy || !status.l3_available}
            onClick={() =>
              activate(
                {
                  release_global_revocation: true,
                  expected_effective_hash: status.effective_hash,
                },
                '解除全局吊销',
              )
            }
          >
            解除全局吊销（需二次认证）
          </button>
        ) : null}
      </section>

      <section className="policy-section" aria-labelledby="policy-desired">
        <h2 id="policy-desired">提交 desired</h2>
        <DesiredForm busy={busy} onSubmit={submit} />
        {selected?.report ? (
          <div className="policy-selected">
            <p>
              <strong>{selected.id}</strong>（{KIND_LABEL[selected.kind]}{' '}
              {selected.subject}，{STATUS_LABEL[selected.status]}）
            </p>
            <PolicyReport report={selected.report} />
            {['pending', 'validated', 'rejected'].includes(selected.status) ? (
              <div className="policy-actions">
                <button
                  className="outline-button"
                  disabled={busy || !selected.report.ok || !status.l3_available}
                  onClick={() =>
                    activate(
                      {
                        desired_id: selected.id,
                        expected_effective_hash: status.effective_hash,
                      },
                      `${KIND_LABEL[selected.kind]} ${selected.subject}`,
                    )
                  }
                >
                  发布（需二次认证）
                </button>
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() => validate(selected)}
                >
                  重新校验
                </button>
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() => withdraw(selected)}
                >
                  撤回
                </button>
              </div>
            ) : null}
          </div>
        ) : null}
      </section>

      <section className="policy-section" aria-labelledby="policy-queue">
        <h2 id="policy-queue">待处理 desired</h2>
        {actionable.length ? (
          <ul className="policy-queue" aria-label="待处理 desired">
            {actionable.map((record) => (
              <li key={record.id}>
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() => setSelected(record)}
                >
                  {KIND_LABEL[record.kind]} {record.subject}
                </button>
                <span>
                  {STATUS_LABEL[record.status]}
                  {record.submitter_kind === 'agent'
                    ? `，由 ${record.agent_type ?? '未知类型'} 实例 ${record.agent_instance ?? '未知'} 提交`
                    : '，主人提交'}
                  ，{when(record.submitted_at)}
                </span>
              </li>
            ))}
          </ul>
        ) : (
          <p className="policy-hint">没有待处理的 desired。</p>
        )}
      </section>

      <section className="policy-section" aria-labelledby="policy-versions">
        <h2 id="policy-versions">版本与回滚</h2>
        {versions.length ? (
          <ul className="policy-versions" aria-label="版本列表">
            {versions.map((meta) => (
              <li key={meta.version}>
                <button
                  className="text-button"
                  disabled={busy}
                  onClick={() => open(meta)}
                >
                  v{meta.version}
                </button>
                <span>
                  {KIND_LABEL[meta.kind]} {meta.subject}
                  {meta.retired ? '（撤销）' : ''}
                  {meta.rollback_of !== null
                    ? `，回滚自 v${meta.rollback_of}`
                    : ''}
                  ，{meta.activated_by}，{when(meta.activated_at)}
                  {meta.expansions.length
                    ? `，${meta.expansions.length} 项扩大`
                    : ''}
                </span>
                <button
                  className="text-button"
                  disabled={busy || !status.l3_available}
                  onClick={() =>
                    activate(
                      {
                        rollback_to: meta.version,
                        expected_effective_hash: status.effective_hash,
                      },
                      `回滚到 v${meta.version}`,
                    )
                  }
                >
                  回滚到此版本（需二次认证）
                </button>
              </li>
            ))}
          </ul>
        ) : (
          <p className="policy-hint">尚无版本。</p>
        )}
        {version ? (
          <div className="policy-version-body">
            <p>
              <strong>
                v{version.meta.version} {KIND_LABEL[version.meta.kind]}{' '}
                {version.meta.subject}
              </strong>{' '}
              sha256{' '}
              <code title={version.meta.body_sha256}>
                {shortHash(version.meta.body_sha256)}
              </code>
            </p>
            <pre>{version.body ?? '（撤销版本，无正文）'}</pre>
          </div>
        ) : null}
      </section>

      <section className="policy-section" aria-labelledby="policy-revoke">
        <h2 id="policy-revoke">三级吊销</h2>
        <RevokeForm busy={busy} onSubmit={revoke} />
      </section>
    </div>
  );
}
