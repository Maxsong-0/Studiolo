import { useState } from 'react';
import type { FormEvent } from 'react';
import type {
  DesiredInput,
  PolicyKind,
  RevokeRequest,
  RevocationLevel,
} from '../policyModel';
import { KIND_LABEL, LEVEL_LABEL } from '../policyModel';

const CAPABILITY_TEMPLATE = JSON.stringify(
  {
    ai_type: 'claude',
    read: {
      project_whitelist: ['项目/Studiolo', '_规则'],
      deny_by_default: true,
    },
    write: { board: false, content_patch_only: true, content_direct: false },
    policy: { propose_desired: true, activate: false },
    hard_bound_skills: [],
  },
  null,
  2,
);
const SKILL_TEMPLATE = JSON.stringify(
  {
    name: 'skill-name',
    summary: '一句摘要',
    triggers: [],
    markdown: '# 正文\n',
  },
  null,
  2,
);

export function DesiredForm({
  busy,
  onSubmit,
}: {
  busy: boolean;
  onSubmit: (input: DesiredInput) => void;
}) {
  const [kind, setKind] = useState<PolicyKind>('capability_set');
  const [retire, setRetire] = useState(false);
  const [error, setError] = useState('');
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    const subject = String(values.get('subject') ?? '').trim();
    const body = String(values.get('body') ?? '');
    if (!subject) {
      setError('请填写 subject（规则文件名、skill 名或 AI 类型名）。');
      return;
    }
    if (!retire && !body.trim()) {
      setError('请填写正文，或勾选「撤销该 subject」。');
      return;
    }
    setError('');
    onSubmit({ kind, subject, body: retire ? null : body });
  }
  return (
    <form className="policy-form" onSubmit={submit} aria-label="提交 desired">
      <div className="policy-form-row">
        <label>
          类别
          <select
            name="kind"
            value={kind}
            onChange={(event) => setKind(event.target.value as PolicyKind)}
          >
            {(Object.keys(KIND_LABEL) as PolicyKind[]).map((value) => (
              <option key={value} value={value}>
                {KIND_LABEL[value]}
              </option>
            ))}
          </select>
        </label>
        <label>
          subject
          <input
            name="subject"
            type="text"
            autoComplete="off"
            spellCheck={false}
            placeholder={
              kind === 'rules'
                ? '通用工作规则.md'
                : kind === 'skills'
                  ? 'vercel-react-best-practices'
                  : 'claude'
            }
          />
        </label>
        <label className="policy-checkbox">
          <input
            type="checkbox"
            checked={retire}
            onChange={(event) => setRetire(event.target.checked)}
          />
          撤销该 subject
        </label>
      </div>
      <label className="policy-body">
        正文（规则为 Markdown，其余为 JSON）
        <textarea
          name="body"
          rows={12}
          spellCheck={false}
          disabled={retire}
          defaultValue={
            kind === 'capability_set'
              ? CAPABILITY_TEMPLATE
              : kind === 'skills'
                ? SKILL_TEMPLATE
                : ''
          }
          key={kind}
        />
      </label>
      {error ? (
        <p className="policy-warning" role="alert">
          {error}
        </p>
      ) : null}
      <div className="policy-actions">
        <button className="outline-button" type="submit" disabled={busy}>
          提交并校验（不发布）
        </button>
        <span className="policy-hint">
          提交只形成待确认意图；发布须先完成二次认证。
        </span>
      </div>
    </form>
  );
}

export function RevokeForm({
  busy,
  onSubmit,
}: {
  busy: boolean;
  onSubmit: (request: RevokeRequest) => void;
}) {
  const [level, setLevel] = useState<RevocationLevel>('instance');
  const [error, setError] = useState('');
  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    const target = String(values.get('target') ?? '').trim();
    const reason = String(values.get('reason') ?? '').trim();
    if (level !== 'global' && !target) {
      setError('请填写要吊销的实例 id 或类型名。');
      return;
    }
    if (!reason) {
      setError('请填写吊销理由（进权威审计）。');
      return;
    }
    setError('');
    onSubmit(
      level === 'global' ? { level, reason } : { level, target, reason },
    );
  }
  return (
    <form className="policy-form" onSubmit={submit} aria-label="吊销">
      <div className="policy-form-row">
        <label>
          级别
          <select
            name="level"
            value={level}
            onChange={(event) =>
              setLevel(event.target.value as RevocationLevel)
            }
          >
            {(Object.keys(LEVEL_LABEL) as RevocationLevel[]).map((value) => (
              <option key={value} value={value}>
                {LEVEL_LABEL[value]}
              </option>
            ))}
          </select>
        </label>
        <label>
          目标
          <input
            name="target"
            type="text"
            autoComplete="off"
            spellCheck={false}
            disabled={level === 'global'}
            placeholder={level === 'type' ? 'claude' : '实例 id（ULID）'}
          />
        </label>
        <label>
          理由
          <input name="reason" type="text" autoComplete="off" />
        </label>
      </div>
      {error ? (
        <p className="policy-warning" role="alert">
          {error}
        </p>
      ) : null}
      <div className="policy-actions">
        <button className="outline-button" type="submit" disabled={busy}>
          吊销（即刻生效）
        </button>
        <span className="policy-hint">
          恢复只走发布：重新发布该类型能力集即解除类型吊销；解除全局吊销需二次认证。
        </span>
      </div>
    </form>
  );
}
