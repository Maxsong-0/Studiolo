import { useId } from 'react';
import { PolicyReport } from '../components/PolicyReport';
import type { ListedDesired, PolicyStatus } from '../policyModel';
import { supersededBy } from '../policyModel';
import { ActionPreview } from '../policy/ActionPreview';
import {
  newerDraftReason,
  publishPreview,
  supersededHint,
} from '../policy/previews';
import { Button } from '../ui';
import { draftOrigin } from './labels';
import type { PolicyActions } from './usePolicyActions';
import '../styles/policy.css';
import '../styles/rules.css';

/**
 * 一份未发布的规则草案（desired）：来历、校验报告（PolicyReport：结论、差异、扩大项）与三个动作——
 * 发布（先展开执行预览，确认才发请求，需二次认证）、校验（按当前生效内容重新校验）、撤回。
 * 同一对象只能发布最新的那份草案（服务端判定，T-181）：这份不是最新就挡住发布并说明；是最新的，
 * 执行预览列出会被一并标为已取代的较早草案。
 * 编辑面板与读权限面板共用；动作与反馈由视图级的 usePolicyActions 统一调度。
 */
export function DraftPanel({
  record,
  title,
  label,
  origin,
  status,
  actions,
  drafts,
  blocked = null,
}: {
  record: ListedDesired;
  /** 块标题，如「最新的未发布草案」「能力集 claude」。 */
  title: string;
  /** 成功提示与执行预览里的对象名，如「规则 CLAUDE.md」。 */
  label: string;
  /** 反馈落在哪一块。 */
  origin: string;
  status: PolicyStatus;
  actions: PolicyActions;
  /** 同一对象全部未发布的草案（含这一份），用来写明发布会取代哪几份。 */
  drafts: readonly ListedDesired[];
  /** 发布之外的原因挡着（如编辑区有未保存的改动）；null 表示没有。 */
  blocked?: string | null;
}) {
  const ids = useId();
  const key = `publish:${record.id}`;
  const previewId = `${ids}-preview`;
  const report = record.report;
  const open = actions.preview?.key === key ? actions.preview : null;
  const superseded = supersededBy(record, drafts);
  const newer = newerDraftReason(record.newer_open);
  // 不是最新的就发不出，「发布会取代哪几份」也就无从谈起，只说为什么发不出。
  const hint = newer ? null : supersededHint(superseded.length);
  const reason = !report
    ? '这份草案还没有校验报告：先校验。'
    : !report.ok
      ? '校验未通过：按逐条问题修改后重新保存，或撤回。'
      : (newer ??
        (!status.l3_available
          ? '本入口没有一次性验证码路径，只能查看与撤回，不能发布。'
          : blocked));
  return (
    <section className="policy-selected" aria-labelledby={`${ids}-title`}>
      <div className="policy-selected__head">
        <h3 className="policy-selected__title" id={`${ids}-title`}>
          {title}
        </h3>
        <p className="policy-selected__meta">
          {draftOrigin(record)} <code>{record.id}</code>
        </p>
      </div>
      {report ? <PolicyReport report={report} /> : null}
      {hint ? <p className="policy-hint">{hint}</p> : null}
      {reason ? <p className="policy-hint">{reason}</p> : null}
      {open ? (
        <ActionPreview
          id={previewId}
          content={open.content}
          disabled={actions.busy}
          onConfirm={actions.confirmPreview}
          onCancel={actions.closePreview}
        />
      ) : null}
      <div className="policy-actions">
        <Button
          variant="danger"
          disabled={actions.busy || reason !== null}
          aria-expanded={open !== null}
          aria-controls={open ? previewId : undefined}
          onClick={() => {
            if (!report) return;
            actions.togglePreview(
              key,
              {
                request: {
                  desired_id: record.id,
                  expected_effective_hash: status.effective_hash,
                },
                label,
                action: `发布 ${label}`,
                origin,
                supersedes: superseded.length,
              },
              publishPreview(record, report, status, superseded),
            );
          }}
        >
          发布（需二次认证）
        </Button>
        <Button
          size="sm"
          disabled={actions.busy}
          onClick={() => actions.validate(origin, record)}
        >
          校验
        </Button>
        <Button
          variant="text"
          size="sm"
          disabled={actions.busy}
          onClick={() => actions.withdraw(origin, record)}
        >
          撤回草案
        </Button>
      </div>
    </section>
  );
}
