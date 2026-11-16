import {
  useDeferredValue,
  useEffect,
  useId,
  useMemo,
  useState,
  type Dispatch,
  type FormEvent,
  type SetStateAction,
} from 'react';
import { FieldLabel } from '../components/PolicyForms';
import type { DesiredInput, PolicyKind, PolicyStatus } from '../policyModel';
import { KIND_LABEL } from '../policyModel';
import { Button, Cell, CellHeader, GridFrame } from '../ui';
import { CapabilityDiff } from './CapabilityDiff';
import { compareCapability } from './capabilityDiff';
import { PolicyFeedback } from './PolicyFeedback';
import { bodyTemplate, startFromBody, type SubmitDraft } from './submitDraft';
import { Term } from './Term';
import { POLICY_TERMS } from './terms';
import type { PolicyActions } from './usePolicyActions';
import type { VersionBodies } from './useVersionBodies';
import '../styles/policy.css';
import '../styles/policy-tabs.css';

/**
 * 策略页「提交」（T-198；设计规范第 9 节）：左边写草案，右边随手列出它和生效中那一版的差别（只对能力集）。
 * 提交只形成待确认的规则草案，永不自动发布；交成功后面板把人带到「待处理」里刚交的那一份。
 * 正在写的内容由面板保管（换标签不丢）；右边那张表只说改了什么，哪些算扩大、收窄由提交后的校验报告判定。
 */
export function PolicySubmit({
  status,
  draft,
  setDraft,
  actions,
  versions,
  onSubmit,
}: {
  status: PolicyStatus;
  draft: SubmitDraft;
  setDraft: Dispatch<SetStateAction<SubmitDraft>>;
  actions: PolicyActions;
  versions: VersionBodies;
  /** 本地校验通过的草案：交给面板提交。 */
  onSubmit: (input: DesiredInput) => void;
}) {
  const id = useId();
  const [error, setError] = useState('');
  const { busy, notice, error: failure } = actions;
  const { bodies, read } = versions;
  const { kind, retire } = draft;
  const subject = draft.subject.trim();
  const capability = kind === 'capability_set';
  // 这个对象现在生效的那一版；没有就是新对象。
  const effective =
    (subject &&
      status.entries.find(
        (entry) => entry.kind === kind && entry.subject === subject,
      )) ||
    null;
  const version = effective?.version ?? null;
  // 能力集要随手比差别，生效的那一版一出现就去读；规则和 skill 等人点「从生效中的版本开始改」再读。
  useEffect(() => {
    if (capability && version !== null) void read(version);
  }, [capability, version, read]);

  const base = version === null ? null : (bodies.get(version) ?? null);
  const baseRecord = base?.kind === 'ready' ? base.record : null;
  // 读到的那一版必须真是这个对象的；对不上就不用它，不拿别的东西凑。
  const mismatched =
    baseRecord !== null &&
    (baseRecord.meta.kind !== kind || baseRecord.meta.subject !== subject);
  const before = baseRecord && !mismatched ? baseRecord.body : null;
  // 右边的表跟着输入走，但让着打字：正文先上屏，表稍后再算。
  const typed = useDeferredValue(draft.body);
  const comparison = useMemo(
    () => compareCapability(before, retire ? null : typed),
    [before, retire, typed],
  );

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!subject) {
      setError('请填写对象（规则文件名、skill 名或 AI 类型名）。');
      return;
    }
    if (!retire && !draft.body.trim()) {
      setError('请填写正文，或勾选「撤销该对象」。');
      return;
    }
    setError('');
    onSubmit({ kind, subject, body: retire ? null : draft.body });
  }

  function startFromEffective() {
    if (!effective) return;
    const target = effective;
    void read(target.version).then((record) => {
      // 没读到、读到的不是这个对象的版本：不填（按钮旁那一句说明原因）。
      if (
        !record ||
        record.body === null ||
        record.meta.kind !== target.kind ||
        record.meta.subject !== target.subject
      )
        return;
      const body = record.body;
      setDraft((current) => startFromBody(current, target, body));
    });
  }

  return (
    <GridFrame className="policy-grid">
      <Cell
        span={6}
        className="policy-cell policy-submit"
        aria-labelledby={`${id}-form`}
      >
        <CellHeader
          title={
            <span id={`${id}-form`}>
              <Term name="desired">提交规则草案</Term>
            </span>
          }
        />
        <PolicyFeedback origin="submit" notice={notice} error={failure} />
        <form
          className="policy-form"
          onSubmit={submit}
          aria-label={`提交${POLICY_TERMS.desired.human}`}
        >
          <div className="policy-form__row">
            <div className="policy-field">
              <FieldLabel htmlFor={`${id}-kind`}>类别</FieldLabel>
              <span className="policy-select">
                <select
                  id={`${id}-kind`}
                  name="kind"
                  value={kind}
                  onChange={(event) => {
                    const next = event.target.value as PolicyKind;
                    // 换类别就换成这一类的起手模板：上一类的正文对新类别没有意义。
                    setDraft((current) => ({
                      ...current,
                      kind: next,
                      body: bodyTemplate(next),
                    }));
                  }}
                >
                  {(Object.keys(KIND_LABEL) as PolicyKind[]).map((value) => (
                    <option key={value} value={value}>
                      {KIND_LABEL[value]}
                    </option>
                  ))}
                </select>
              </span>
            </div>
            <div className="policy-field">
              <FieldLabel
                htmlFor={`${id}-subject`}
                raw={POLICY_TERMS.subject.raw}
              >
                {POLICY_TERMS.subject.human}
              </FieldLabel>
              <input
                id={`${id}-subject`}
                className="policy-input policy-input--mono"
                name="subject"
                type="text"
                autoComplete="off"
                spellCheck={false}
                value={draft.subject}
                onChange={(event) => {
                  const next = event.target.value;
                  setDraft((current) => ({ ...current, subject: next }));
                }}
                placeholder={
                  kind === 'rules'
                    ? '通用工作规则.md'
                    : kind === 'skills'
                      ? 'vercel-react-best-practices'
                      : 'claude'
                }
              />
            </div>
          </div>
          <label className="policy-check">
            <input
              type="checkbox"
              checked={retire}
              onChange={(event) => {
                const next = event.target.checked;
                setDraft((current) => ({ ...current, retire: next }));
              }}
            />
            撤销该对象
          </label>
          <div className="policy-field">
            <FieldLabel htmlFor={`${id}-body`}>
              正文（规则为 Markdown，其余为 JSON）
            </FieldLabel>
            <textarea
              id={`${id}-body`}
              className="policy-input policy-textarea"
              name="body"
              rows={14}
              spellCheck={false}
              disabled={retire}
              value={draft.body}
              onChange={(event) => {
                const next = event.target.value;
                setDraft((current) => ({ ...current, body: next }));
              }}
            />
          </div>
          {effective ? (
            <p className="policy-submit__base">
              <Button
                size="sm"
                disabled={retire || base?.kind === 'loading' || mismatched}
                onClick={startFromEffective}
              >
                从生效中的 v{effective.version} 开始改
              </Button>
              <span className="policy-hint">
                {base?.kind === 'failed'
                  ? `没读到 v${effective.version} 的正文，再点一次重读。`
                  : mismatched
                    ? `读到的 v${effective.version} 不是这个对象的版本，没有填入。`
                    : '把现在生效的正文放进上面，覆盖已经写的内容。'}
              </span>
            </p>
          ) : null}
          {error ? (
            <p className="policy-form__error" role="alert">
              {error}
            </p>
          ) : null}
          <div className="policy-actions">
            <Button type="submit" variant="primary" disabled={busy}>
              提交并校验（不发布）
            </Button>
            <span className="policy-hint">
              提交只形成待确认的规则草案；发布须先完成二次认证。
            </span>
          </div>
        </form>
      </Cell>
      <Cell
        span={6}
        className="policy-cell policy-compare"
        aria-labelledby={`${id}-compare`}
      >
        <CellHeader
          title={<span id={`${id}-compare`}>和生效中的差别</span>}
          meta={
            capability && subject
              ? version === null
                ? '没有生效版本'
                : `生效中 v${version}`
              : undefined
          }
        />
        {!capability ? (
          <p className="policy-hint">
            这里只对能力集列出「现在／改后」。规则和 skill
            的差别，提交后在校验报告里逐项列出。
          </p>
        ) : !subject ? (
          <p className="policy-hint">
            先填对象（AI 类型名）：这里会列出它和生效中那一版的差别。
          </p>
        ) : (
          <>
            {retire ? (
              <p className="policy-hint">
                撤销：发布后这类 AI 回到零权限，下面「现在」一列的能力都会收回。
              </p>
            ) : version === null ? (
              <p className="policy-hint">
                这类 AI
                现在没有生效的能力集：发布后，下面「改后」一列的每一项都是新给的。
              </p>
            ) : null}
            {version !== null && base?.kind === 'failed' ? (
              <p className="policy-hint">
                没读到生效中 v{version} 的正文，这里列不出差别；提交不受影响。{' '}
                <Button
                  variant="text"
                  size="sm"
                  onClick={() => void read(version)}
                >
                  再读一次
                </Button>
              </p>
            ) : version !== null && baseRecord === null ? (
              <p className="policy-hint" role="status">
                正在读生效中 v{version} 的正文…
              </p>
            ) : mismatched ? (
              <p className="policy-hint">
                读到的 v{version} 不是这类 AI
                的能力集，这里列不出差别；提交后看校验报告。
              </p>
            ) : (
              <CapabilityDiff
                comparison={comparison}
                before={before === null ? '没有生效版本' : `生效中 v${version}`}
                after={retire ? '撤销之后' : '正在写的'}
                typing
              />
            )}
            <p className="policy-hint">
              这张表只列改了什么；哪些算扩大、哪些算收窄，由提交后的校验报告判定。
            </p>
          </>
        )}
      </Cell>
    </GridFrame>
  );
}
