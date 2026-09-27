import { useId, useMemo } from 'react';
import type { ReactNode } from 'react';
import { PolicyStepUp } from '../components/PolicyStepUp';
import { RuleEditor } from '../rules/RuleEditor';
import { RulesAccess } from '../rules/RulesAccess';
import { RulesList } from '../rules/RulesList';
import { buildRulesModel } from '../rules/rulesModel';
import { usePolicyActions } from '../rules/usePolicyActions';
import { useRulesData } from '../rules/useRulesData';
import { Cell, EmptyState, ErrorState, GridFrame, Skeleton, Tabs } from '../ui';
import { useViewLocation } from '../useViewLocation';
import '../styles/rules.css';

const panels = [
  { id: 'list', label: '列表' },
  { id: 'edit', label: '编辑' },
  { id: 'access', label: '读权限' },
] as const;

type RulesPanel = (typeof panels)[number]['id'];

/** 默认面板「列表」记作 null；未知面板值一律当列表，不猜。 */
function rulesPanel(value: string | null): RulesPanel {
  return value === 'edit' || value === 'access' ? value : 'list';
}

/** 编辑器按规则换实例：新建记作 `new`，已有规则带 `rule:` 前缀，两者不会撞。 */
function editorKey(item: string | null): string {
  return item === null ? 'new' : `rule:${item}`;
}

/** 读取中、未接入、读取失败都占满一个 12 列的格子，与就绪后的网格同一版式。 */
function StateCell({ children }: { children: ReactNode }) {
  return (
    <GridFrame className="rules-grid">
      <Cell span={12} className="rules-cell" aria-label="规则">
        {children}
      </Cell>
    </GridFrame>
  );
}

/**
 * 规则（T-160，ADR-026）：顶栏第五项。三个面板——列表（默认）、编辑（item＝规则名，空为新建）、读权限矩阵。
 * 规则的唯一母本是策略平面 rules 类的生效版本；人侧永远全读，读权限只约束 AI（能力集 read.rules_whitelist）。
 * 不依赖工作台快照：快照读不出来时也要能看规则、提交草案与发布。发布被要求二次认证时就地弹窗、认证后原样重试。
 */
export function RulesView() {
  const { location, goTo } = useViewLocation();
  const panel = rulesPanel(location.panel);
  const id = useId();
  const { data, reload, retry } = useRulesData();
  const actions = usePolicyActions(reload);
  const snapshot = data.kind === 'ready' ? data.snapshot : null;
  const model = useMemo(
    () => (snapshot ? buildRulesModel(snapshot) : null),
    [snapshot],
  );

  // 换面板只换 panel，在编辑的规则留在 item 里（与协作的子标签同一做法）。
  const show = (next: RulesPanel, item: string | null = location.item) =>
    goTo({ view: 'rules', panel: next === 'list' ? null : next, item });

  let body: ReactNode;
  if (data.kind === 'loading')
    body = (
      <StateCell>
        <Skeleton label="正在读取规则…" lines={4} />
      </StateCell>
    );
  else if (data.kind === 'absent')
    body = (
      <StateCell>
        <EmptyState title="未接入策略平面">
          <p>
            本入口未装配策略平面（启动时未给
            --policy-data）。规则母本只能在装配了策略平面的会话入口查看与发布。
          </p>
        </EmptyState>
      </StateCell>
    );
  else if (data.kind === 'error' || !model || !snapshot)
    body = (
      <StateCell>
        <ErrorState title="读取规则失败" retryLabel="重新读取" onRetry={retry}>
          <p>{data.kind === 'error' ? data.message : '规则数据不完整。'}</p>
        </ErrorState>
      </StateCell>
    );
  else if (panel === 'list')
    body = (
      <RulesList
        model={model}
        status={snapshot.status}
        onOpen={(subject) => show('edit', subject)}
        onCreate={() => show('edit', null)}
      />
    );
  else if (panel === 'edit')
    body = (
      <RuleEditor
        key={editorKey(location.item)}
        subject={location.item}
        model={model}
        status={snapshot.status}
        actions={actions}
        onOpen={(subject) => show('edit', subject)}
        onCreated={(subject) => show('edit', subject)}
        onBack={() => show('list')}
      />
    );
  else
    body = (
      <RulesAccess model={model} status={snapshot.status} actions={actions} />
    );

  return (
    <div className="view view-rules rules">
      <h1 className="sr-only">规则</h1>
      <div className="rules-bar">
        <Tabs
          items={panels}
          value={panel}
          onChange={(next) => show(rulesPanel(next))}
          ariaLabel="规则面板"
          idPrefix={id}
          className="rules-bar__tabs"
        />
        {model ? (
          <p className="rules-bar__summary">
            <span>{model.rules.length} 条规则</span>
            <span>{model.types.length} 类 AI</span>
          </p>
        ) : null}
      </div>
      <div
        className="rules-panel"
        role="tabpanel"
        id={`${id}-panel-${panel}`}
        aria-labelledby={`${id}-tab-${panel}`}
      >
        {body}
      </div>
      {actions.pending ? (
        <PolicyStepUp
          action={actions.pending.action}
          attempt={actions.pending.attempt}
          onAuthenticated={actions.retryAfterStepUp}
          onClose={actions.closeStepUp}
        />
      ) : null}
    </div>
  );
}
