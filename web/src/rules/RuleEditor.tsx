import { lazy, useDeferredValue, useId, useState } from 'react';
import { EditorPane } from '../components/EditorPane';
import type { PolicyStatus } from '../policyModel';
import { Button, Cell, CellHeader, GridFrame } from '../ui';
import { DraftPanel } from './DraftPanel';
import { displayTime } from './labels';
import { RulesFeedback } from './RulesFeedback';
import {
  RULE_BODY_MAX_BYTES,
  bodyBytes,
  ruleSubjectProblem,
} from './rulesModel';
import type { RulesModel } from './rulesModel';
import type { PolicyActions } from './usePolicyActions';
import '../styles/policy.css';
import '../styles/rules.css';

// 源码编辑器与预览按需加载：CodeMirror 与 remark 不进首屏（与 CanonicalEditor 同一做法）。
const MarkdownEditor = lazy(() =>
  import('../components/MarkdownEditor').then((module) => ({
    default: module.MarkdownEditor,
  })),
);
const MarkdownPreview = lazy(() =>
  import('../components/MarkdownPreview').then((module) => ({
    default: module.MarkdownPreview,
  })),
);

const ORIGIN = 'editor';
const ignore = () => undefined;
const withinLimit = (value: string) => bodyBytes(value) <= RULE_BODY_MAX_BYTES;

/**
 * 编辑面板：左源码右预览（MarkdownEditor 受控＋MarkdownPreview）。
 * 编辑已有规则时以最新的未发布草案为底（没有就用生效正文）；新建时先填规则名，前端按后端同一口径预校验。
 * 保存＝提交规则草案（desired，不发布）；草案的校验报告、校验、发布（执行预览＋二次认证）与撤回在下方一格。
 */
export function RuleEditor({
  subject,
  model,
  status,
  actions,
  onOpen,
  onCreated,
  onBack,
}: {
  /** 位置里的规则名；null 表示新建。 */
  subject: string | null;
  model: RulesModel;
  status: PolicyStatus;
  actions: PolicyActions;
  onOpen: (subject: string) => void;
  /** 新规则保存成功（并已重读）后改写位置，从此按「编辑已有规则」显示。 */
  onCreated: (subject: string) => void;
  onBack: () => void;
}) {
  const ids = useId();
  const row = model.rules.find((rule) => rule.subject === subject);
  const existing =
    row !== undefined && (row.entry !== null || row.drafts.length > 0);
  const draft = existing ? (row.drafts[0] ?? null) : null;
  const base = draft?.body ?? row?.body ?? '';
  const [name, setName] = useState(subject ?? '');
  const [source, setSource] = useState(base);
  const [seen, setSeen] = useState(base);
  if (base !== seen) {
    // 底换了（保存、发布或别处改动后重读）：编辑区没动过就跟上新的底，动过就留着人手上的改动。
    setSeen(base);
    if (source === seen) setSource(base);
  }
  const preview = useDeferredValue(source);
  const [tried, setTried] = useState(false);
  const [limited, setLimited] = useState(false);
  const [local, setLocal] = useState<string | null>(null);

  const target = existing ? row.subject : name.trim();
  const problem = existing ? null : ruleSubjectProblem(target);
  const taken =
    !existing &&
    model.rules.some(
      (rule) =>
        rule.subject === target &&
        (rule.entry !== null || rule.drafts.length > 0),
    );
  const dirty = source !== base;
  const showProblem = problem !== null && (tried || name !== '');

  function save() {
    setLocal(null);
    if (!existing) {
      setTried(true);
      if (problem || taken) return;
    }
    if (!source.trim()) {
      setLocal('正文不能为空：规则母本至少要有一行内容。');
      return;
    }
    actions.submit(
      ORIGIN,
      { kind: 'rules', subject: target, body: source },
      existing ? undefined : () => onCreated(target),
    );
  }

  const meta = !existing
    ? '保存后成为规则草案，发布才生效'
    : row.entry
      ? `生效 v${row.entry.version} · ${displayTime(row.entry.activated_at)} · 读者 ${
          row.readers.length ? row.readers.join('、') : '无'
        }`
      : '还没有生效版本';
  return (
    <GridFrame className="rules-grid">
      <Cell span={12} className="rules-cell" aria-labelledby={`${ids}-title`}>
        <CellHeader
          title={
            <span id={`${ids}-title`}>
              {existing ? `编辑 ${target}` : '新建规则'}
            </span>
          }
          meta={meta}
          action={
            <Button variant="text" size="sm" onClick={onBack}>
              回到列表
            </Button>
          }
        />
        {existing ? null : (
          <div className="policy-field rules-name">
            <span className="policy-field__label">
              <label htmlFor={`${ids}-name`}>规则名</label>{' '}
              <code className="policy-term" lang="en">
                subject
              </code>
            </span>
            <input
              id={`${ids}-name`}
              className="policy-input policy-input--mono"
              value={name}
              onChange={(event) => setName(event.target.value)}
              autoComplete="off"
              spellCheck={false}
              placeholder="通用工作规则.md"
              aria-invalid={showProblem || taken ? true : undefined}
              aria-describedby={`${ids}-name-hint`}
            />
            <p className="policy-hint" id={`${ids}-name-hint`}>
              单个 Markdown 文件名，就是规则名：以 .md 结尾、不超过 128
              个字符、不含 /，也不以 . 开头。
            </p>
            {showProblem ? (
              <p className="policy-form__error" role="alert">
                {problem}
              </p>
            ) : null}
            {taken ? (
              <p className="policy-form__error" role="alert">
                已有同名规则「{target}」。
                <Button variant="text" size="sm" onClick={() => onOpen(target)}>
                  去编辑它
                </Button>
              </p>
            ) : null}
          </div>
        )}
        <RulesFeedback
          origin={ORIGIN}
          notice={actions.notice}
          error={actions.error}
        />
        {local ? (
          <p className="policy-form__error" role="alert">
            {local}
          </p>
        ) : null}
        {limited ? (
          <p className="policy-hint" role="status">
            正文已到 256 KiB 上限，超出的输入没有写进去。
          </p>
        ) : null}
        <div className="policy-actions">
          <Button
            variant="primary"
            disabled={actions.busy || (existing && !dirty)}
            onClick={save}
          >
            保存为规则草案（不发布）
          </Button>
          <span className="policy-hint">
            {dirty
              ? '有未保存的改动。'
              : existing
                ? draft
                  ? '编辑区与最新草案一致。'
                  : '编辑区与生效版本一致。'
                : '写好正文后保存。'}
          </span>
        </div>
      </Cell>
      <Cell span={6} flush className="rules-source" aria-label="规则源码">
        <EditorPane content={source} label="源码编辑器">
          <MarkdownEditor
            value={source}
            readOnly={false}
            onChange={(value) => {
              setSource(value);
              setLimited(false);
            }}
            canChange={withinLimit}
            onLimit={() => setLimited(true)}
            onBlur={ignore}
            onComposition={ignore}
          />
        </EditorPane>
      </Cell>
      <Cell span={6} as="div" className="rules-preview">
        <EditorPane content={source} label="Markdown 预览">
          <MarkdownPreview content={preview} updating={preview !== source} />
        </EditorPane>
      </Cell>
      <Cell span={12} className="rules-cell" aria-labelledby={`${ids}-draft`}>
        <CellHeader
          title={<span id={`${ids}-draft`}>草案与发布</span>}
          meta={existing ? `未发布草案 ${row.drafts.length} 份` : undefined}
        />
        {draft ? (
          <DraftPanel
            record={draft}
            title="最新的未发布草案"
            label={`规则 ${target}`}
            origin={ORIGIN}
            status={status}
            actions={actions}
            others={row ? row.drafts.length - 1 : 0}
            blocked={
              dirty
                ? '编辑区有未保存的改动：先保存为新的规则草案，再发布。'
                : null
            }
          />
        ) : (
          <p className="policy-hint">
            还没有未发布的草案：保存后这里出现校验报告（差异与扩大项），校验通过才能发布。
          </p>
        )}
      </Cell>
    </GridFrame>
  );
}
