import { useId } from 'react';
import { COMMAND_TARGETS } from '../commands/focusTarget';
import { RevokeForm } from '../components/PolicyForms';
import { ChangeList } from '../components/PolicyReport';
import type { PolicyStatus, VersionMeta, VersionRecord } from '../policyModel';
import { KIND_LABEL, LEVEL_LABEL, shortHash } from '../policyModel';
import { Button, Cell, CellHeader, GridFrame } from '../ui';
import { cx } from '../ui/dom';
import { ActionPreview } from './ActionPreview';
import { displayTime } from './labels';
import { PolicyFeedback } from './PolicyFeedback';
import { releasePreview, rollbackPreview } from './previews';
import { Term } from './Term';
import { POLICY_TERMS } from './terms';
import { REVOKE_PREVIEW, type PolicyActions } from './usePolicyActions';
import '../styles/policy.css';

/**
 * 策略页「现状」（T-198；设计规范第 9 节）：现在生效的是什么，以及对它动手的三件事——
 * 第一行生效中各项、生效中的撤销接入、全局撤销的解除；第二行版本与回滚 ｜ 撤销接入。
 * 回滚、解除全局撤销、撤销接入都先在按钮上方展开执行预览，确认才发请求；动作和反馈由面板那一份
 * usePolicyActions 统一调度，反馈落在发起动作的那一格。
 */
export function PolicyNow({
  status,
  versions,
  actions,
  reading,
  onOpenVersion,
}: {
  status: PolicyStatus;
  versions: readonly VersionMeta[];
  actions: PolicyActions;
  /** 在读的那一版正文（位置里的版本号对得上、正文也读到了）；没有就是 null。 */
  reading: VersionRecord | null;
  onOpenVersion: (meta: VersionMeta) => void;
}) {
  const ids = useId();
  // 同一时刻只有一个执行预览，一个 id 就够：触发按钮展开时用 aria-controls 指向它。
  const previewId = `${ids}-preview`;
  const { busy, notice, error, preview } = actions;
  const feedback = { notice, error };
  const heading = {
    effective: `${ids}-effective`,
    versions: `${ids}-versions`,
    revoke: `${ids}-revoke`,
  };

  function previewFor(key: string) {
    if (preview?.key !== key) return null;
    return (
      <ActionPreview
        id={previewId}
        content={preview.content}
        disabled={busy}
        onConfirm={actions.confirmPreview}
        onCancel={actions.closePreview}
      />
    );
  }
  /** 触发按钮与它的预览框的关联：展开时指向框。 */
  function trigger(key: string) {
    const expanded = preview?.key === key;
    return {
      'aria-expanded': expanded,
      'aria-controls': expanded ? previewId : undefined,
    };
  }

  return (
    <GridFrame className="policy-grid">
      {/* ---------- 第一行：生效中 ---------- */}
      <Cell
        span={12}
        className="policy-cell policy-effective"
        aria-labelledby={heading.effective}
      >
        <CellHeader
          title={
            <span id={heading.effective}>
              <Term name="effective" />
            </span>
          }
          meta={
            <>
              {POLICY_TERMS.sha256.human}{' '}
              <code className="policy-hash" title={status.effective_hash}>
                {shortHash(status.effective_hash)}
              </code>
            </>
          }
        />
        <PolicyFeedback origin="effective" {...feedback} />
        <p className="policy-note">
          待处理草案 {status.pending_desired} 件；
          {status.l3_available
            ? '本入口可以经一次性验证码发布。'
            : '本入口没有一次性验证码路径，只能查看与撤销接入。'}
        </p>
        {status.global_revoked ? (
          <div className="policy-global">
            <p className="policy-global__alert" role="alert">
              全局撤销生效中：所有 AI 都被拒绝。解除需要二次认证。
            </p>
            {previewFor('release')}
            <div className="policy-actions">
              <Button
                variant="danger"
                disabled={busy || !status.l3_available}
                {...trigger('release')}
                onClick={() =>
                  actions.togglePreview(
                    'release',
                    {
                      request: {
                        release_global_revocation: true,
                        expected_effective_hash: status.effective_hash,
                      },
                      label: '解除全局撤销',
                      action: '解除全局撤销',
                      origin: 'effective',
                      supersedes: 0,
                      remains: 0,
                    },
                    releasePreview(status.effective_hash),
                  )
                }
              >
                解除全局撤销（需二次认证）
              </Button>
            </div>
          </div>
        ) : null}
        <div className="policy-effective__body">
          <table className="policy-table" aria-label="生效中的各项">
            <thead>
              <tr>
                <th scope="col">类别</th>
                <th scope="col">
                  <Term name="subject" />
                </th>
                <th scope="col" className="is-number">
                  版本
                </th>
                <th scope="col" className="is-number">
                  <Term name="sha256" />
                </th>
                <th scope="col" className="is-number">
                  发布时间
                </th>
              </tr>
            </thead>
            <tbody>
              {status.entries.length ? (
                status.entries.map((entry) => (
                  <tr key={`${entry.kind}:${entry.subject}`}>
                    <td className="policy-table__kind">
                      {KIND_LABEL[entry.kind]}
                    </td>
                    <td className="policy-table__subject">{entry.subject}</td>
                    <td className="is-number">v{entry.version}</td>
                    <td className="is-number">
                      <code title={entry.body_sha256}>
                        {shortHash(entry.body_sha256)}
                      </code>
                    </td>
                    <td className="is-number">
                      {displayTime(entry.activated_at)}
                    </td>
                  </tr>
                ))
              ) : (
                <tr>
                  <td colSpan={5} className="policy-table__empty">
                    尚无任何生效母本：所有 AI 类型都是零权限，直到发布其能力集。
                  </td>
                </tr>
              )}
            </tbody>
          </table>
          <div className="policy-revocations">
            <h3 className="policy-subhead">
              生效中的撤销接入
              <span className="policy-subhead__count">
                {status.revocations.length}
              </span>
            </h3>
            {status.revocations.length ? (
              <ul aria-label="生效中的撤销接入">
                {status.revocations.map((item) => (
                  <li
                    key={item.seq}
                    className={cx(item.level === 'global' && 'is-global')}
                  >
                    <span className="policy-revocations__level">
                      {item.level === 'global' ? (
                        <span className="ui-signal-mark" aria-hidden="true" />
                      ) : null}
                      {LEVEL_LABEL[item.level]}
                    </span>
                    <span className="policy-revocations__what">
                      <code>{item.target}</code>
                      <span className="policy-revocations__reason">
                        {item.reason}
                      </span>
                    </span>
                    <span className="policy-revocations__when">
                      {displayTime(item.revoked_at)}
                    </span>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="policy-hint">没有生效中的撤销接入。</p>
            )}
          </div>
        </div>
      </Cell>

      {/* ---------- 第二行：版本与回滚 ｜ 撤销接入 ---------- */}
      <Cell
        span={7}
        tabIndex={-1}
        data-command-target={COMMAND_TARGETS.policyVersions}
        className="policy-cell policy-versions"
        aria-labelledby={heading.versions}
      >
        <CellHeader
          title={
            <span id={heading.versions}>
              版本与<Term name="rollback">回滚</Term>
            </span>
          }
          meta={`${versions.length} 版`}
        />
        <PolicyFeedback origin="versions" {...feedback} />
        {versions.length ? (
          <ul className="policy-version-list" aria-label="版本列表">
            {versions.map((meta) => {
              const key = `rollback:${meta.version}`;
              return (
                <li key={meta.version} className="policy-version">
                  <button
                    type="button"
                    className="policy-version__open"
                    aria-current={
                      reading?.meta.version === meta.version
                        ? 'true'
                        : undefined
                    }
                    disabled={busy}
                    onClick={() => onOpenVersion(meta)}
                  >
                    v{meta.version}
                  </button>
                  <span className="policy-version__what">
                    <span className="policy-version__subject">
                      {KIND_LABEL[meta.kind]} {meta.subject}
                      {meta.retired ? '（撤销）' : ''}
                    </span>
                    {/* 发布时的扩大项是已经确认过的历史，不等人也不是故障：写成中性色文字，不标红 */}
                    <span className="policy-version__meta">
                      {meta.rollback_of !== null
                        ? `回滚自 v${meta.rollback_of} · `
                        : ''}
                      {meta.activated_by} · {displayTime(meta.activated_at)}
                      {meta.expansions.length ? (
                        <>
                          {' · '}
                          <span className="policy-nowrap">
                            发布时扩大 {meta.expansions.length} 项
                          </span>
                        </>
                      ) : null}
                    </span>
                  </span>
                  {previewFor(key)}
                  <Button
                    variant="danger"
                    size="sm"
                    className="policy-version__rollback"
                    disabled={busy || !status.l3_available}
                    {...trigger(key)}
                    onClick={() =>
                      actions.togglePreview(
                        key,
                        {
                          request: {
                            rollback_to: meta.version,
                            expected_effective_hash: status.effective_hash,
                          },
                          label: `回滚到 v${meta.version}`,
                          action: `回滚到 v${meta.version}`,
                          origin: 'versions',
                          supersedes: 0,
                          remains: 0,
                        },
                        rollbackPreview(meta, status),
                      )
                    }
                  >
                    回滚到此版本（需二次认证）
                  </Button>
                </li>
              );
            })}
          </ul>
        ) : (
          <p className="policy-hint">尚无版本。</p>
        )}
        {reading ? (
          <div className="policy-version-body">
            <div className="policy-version-body__head">
              <h3 className="policy-subhead">
                v{reading.meta.version} {KIND_LABEL[reading.meta.kind]}{' '}
                {reading.meta.subject}
              </h3>
              <p className="policy-version-body__hash">
                {POLICY_TERMS.sha256.human}{' '}
                <code title={reading.meta.body_sha256}>
                  {shortHash(reading.meta.body_sha256)}
                </code>
              </p>
            </div>
            <pre>{reading.body ?? '（撤销版本，无正文）'}</pre>
            <p className="policy-version-body__label">
              发布时记录的扩大项（相对上一份生效中）
            </p>
            {reading.meta.expansions.length ? (
              <ChangeList
                items={reading.meta.expansions}
                kind="expansion"
                historical
                label={`v${reading.meta.version} 发布时的扩大项`}
              />
            ) : (
              <p className="policy-hint">这一版发布时没有记录扩大项。</p>
            )}
            <p className="policy-hint">
              版本记录只保存发布时的扩大项；逐项差异只在规则草案的校验报告里提供。
            </p>
          </div>
        ) : null}
      </Cell>
      <Cell
        span={5}
        className="policy-cell policy-revoke"
        aria-labelledby={heading.revoke}
      >
        <CellHeader
          title={
            <span id={heading.revoke}>
              <Term name="revoke">撤销接入（实例/类型/全局）</Term>
            </span>
          }
        />
        <PolicyFeedback origin="revoke" {...feedback} />
        <RevokeForm
          busy={busy}
          preview={previewFor(REVOKE_PREVIEW)}
          previewId={previewId}
          onRequest={(request) => actions.previewRevoke('revoke', request)}
          onEdit={() =>
            actions.closePreviewIf((open) => open.kind === 'revoke')
          }
        />
      </Cell>
    </GridFrame>
  );
}
