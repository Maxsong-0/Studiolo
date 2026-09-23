import { useEffect, useRef, useState } from 'react';
import { useViewLocation } from './useViewLocation';
import type { ReactNode } from 'react';
import { CandidateDetails } from './ExecutionCandidates';
import { LocalDispatch } from './LocalDispatch';
import { boardLease, boardTree } from './boardModel';
import type { BoardItem, BoardNode, BoardStatus } from './boardModel';
import {
  Button,
  Cell,
  CellHeader,
  EmptyState,
  ErrorState,
  GridFrame,
  Skeleton,
  StatusMark,
} from './ui';
import type { StatusState } from './ui';
import { cx } from './ui/dom';
import {
  BOARD_REQUEST_MS,
  boardFailure,
  fetchBoardItems,
} from './collab/boardDetailApi';
import {
  assessmentLabels,
  receiptLabels,
  statusLabels,
  statusMark,
} from './collab/boardLabels';
import { ProjectSelect } from './collab/ProjectSelect';
import './styles/board.css';

type State =
  | { kind: 'loading' }
  | { kind: 'ready'; items: BoardItem[]; at: number }
  | { kind: 'error'; message: string };
const REFRESH_MS = 15_000;
const STALE_MS = 30_000;
const terminal = new Set<BoardStatus>([
  'succeeded',
  'failed',
  'cancelled',
  'abandoned',
  'needs_review',
  'approved',
]);

export function BoardBoundary({
  projects,
}: {
  projects: { root: string; name: string }[];
}) {
  const { location, select } = useViewLocation();
  const selected = location.project ?? '';
  const project = projects.some((p) => p.root === selected)
    ? selected
    : (projects[0]?.root ?? '');
  return (
    <section className="board-panel board" aria-label="协作黑板">
      <GridFrame className="board-grid">
        <Cell className="board-bar">
          <div className="board-toolbar">
            <ProjectSelect
              value={project || null}
              options={projects}
              onChange={(root) => {
                if (root) select({ project: root, item: null });
              }}
            />
            <span>状态核对 · 每 15 秒刷新</span>
            {/* 回放是协作页的「时间线」子标签：切过去，项目格原样带着。 */}
            <Button size="sm" onClick={() => select({ panel: 'timeline' })}>
              项目时间回放
            </Button>
          </div>
        </Cell>
        {/* 本机派发与黑板轮询是兄弟节点：各用各的 key，换项目时两边一起重挂。 */}
        {project ? (
          <Cell className="board-dispatch">
            <LocalDispatch key={`dispatch:${project}`} project={project} />
          </Cell>
        ) : null}
        {project ? (
          <BoardProject
            key={`board:${project}`}
            project={project}
            focusId={location.item}
          />
        ) : (
          <Cell>
            <EmptyState title="当前没有可查看的项目。" />
          </Cell>
        )}
      </GridFrame>
    </section>
  );
}
function BoardProject({
  project,
  focusId,
}: {
  project: string;
  focusId: string | null;
}) {
  const [state, setState] = useState<State>({ kind: 'loading' });
  const [generation, setGeneration] = useState(0);
  useEffect(() => {
    let disposed = false;
    let controller: AbortController | undefined;
    let timer: ReturnType<typeof setTimeout> | undefined;
    let deadline: ReturnType<typeof setTimeout> | undefined;
    function schedule() {
      if (!disposed) timer = setTimeout(() => void load(), REFRESH_MS);
    }
    async function load() {
      const request = new AbortController();
      controller = request;
      let expired = false;
      const timeout = setTimeout(() => {
        expired = true;
        request.abort();
        if (!disposed) {
          setState({
            kind: 'error',
            message: '黑板读取超时，连接状态已失效。',
          });
          schedule();
        }
      }, BOARD_REQUEST_MS);
      deadline = timeout;
      try {
        const items = await fetchBoardItems(project, request.signal);
        if (disposed || expired || request.signal.aborted) return;
        setState({ kind: 'ready', items, at: Date.now() });
      } catch (error) {
        if (disposed || expired || request.signal.aborted) return;
        setState({ kind: 'error', message: boardFailure(error) });
      } finally {
        clearTimeout(timeout);
        if (!expired) schedule();
      }
    }
    void load();
    return () => {
      disposed = true;
      controller?.abort();
      clearTimeout(timer);
      clearTimeout(deadline);
    };
  }, [project, generation]);
  function retry() {
    setState({ kind: 'loading' });
    setGeneration((n) => n + 1);
  }
  if (state.kind === 'loading')
    return (
      <Cell>
        <Skeleton label="正在读取协作状态…" />
      </Cell>
    );
  if (state.kind === 'error')
    return (
      <Cell>
        <Unavailable message={state.message} retry={retry} />
      </Cell>
    );
  return (
    <BoardSnapshot
      items={state.items}
      at={state.at}
      retry={retry}
      focusId={focusId}
    />
  );
}
function Unavailable({
  message,
  retry,
}: {
  message: string;
  retry: () => void;
}) {
  return (
    <ErrorState
      className="board-unavailable"
      title={
        <span role="heading" aria-level={3}>
          实时状态不可确认
        </span>
      }
      onRetry={retry}
      retryLabel="重新核对"
    >
      <p>{message}</p>
      <p>暂停依据旧状态进行认领和写入。</p>
    </ErrorState>
  );
}
function BoardSnapshot({
  items,
  at,
  retry,
  focusId,
}: {
  items: BoardItem[];
  at: number;
  retry: () => void;
  focusId: string | null;
}) {
  const [clock, setClock] = useState(Date.now);
  useEffect(() => {
    const current = Date.now();
    if (current >= at + STALE_MS) return;
    const deadlines = items.flatMap((item) =>
      item.attempt &&
      ['claimed', 'running'].includes(item.attempt.phase) &&
      item.attempt.active &&
      item.attempt.leaseUntilMs > current
        ? [item.attempt.leaseUntilMs]
        : [],
    );
    const next = Math.min(at + STALE_MS, ...deadlines);
    const timer = setTimeout(
      () => setClock(Date.now()),
      Math.max(1, next - current),
    );
    return () => clearTimeout(timer);
  }, [at, items, clock]);
  const now = Math.max(clock, at);
  if (now - at >= STALE_MS)
    return (
      <Cell>
        <Unavailable
          message="上次核对已超过 30 秒，当前连接状态不可确认。"
          retry={retry}
        />
      </Cell>
    );
  const active = items.filter(
    (item) => boardLease(item, now) === 'current',
  ).length;
  return (
    <Cell className="board-items">
      <CellHeader
        title="工作项"
        meta={
          <span className="board-freshness">
            已核对{' '}
            <time dateTime={new Date(at).toISOString()}>
              {new Date(at).toLocaleTimeString()}
            </time>{' '}
            · {items.length} 个工作项 · {active} 项租约有效
          </span>
        }
      />
      <p className="board-hint">
        租约时间按本机时钟显示；认领与写入以服务器当次裁决为准。
      </p>
      <InstanceTree items={items} />
      {items.length ? (
        <ol className="board-tree" aria-label="工作项树">
          {boardTree(items).map((node) => (
            <TaskNode
              key={node.item.id}
              node={node}
              now={now}
              focusId={focusId}
            />
          ))}
        </ol>
      ) : (
        <EmptyState
          className="board-empty"
          title={
            <span role="heading" aria-level={3}>
              无活动
            </span>
          }
        >
          <p>此项目当前没有黑板工作项。</p>
        </EmptyState>
      )}
    </Cell>
  );
}
/** 状态方块：租约到期算出问题；排队但子项未收尾算待定。 */
function nodeMark(
  item: BoardItem,
  lease: ReturnType<typeof boardLease>,
): StatusState {
  if (lease === 'expired') return 'alert';
  return statusMark(item.status);
}
function TaskNode({
  node,
  now,
  focusId,
}: {
  node: BoardNode;
  now: number;
  focusId: string | null;
}) {
  const { item, children } = node;
  const lease = boardLease(item, now);
  const blocked = children.some((child) => !terminal.has(child.item.status));
  const label =
    item.status === 'queued' && blocked
      ? '等待子项收尾'
      : lease === 'expired'
        ? '租约已到期，状态待核对'
        : statusLabels[item.status];
  // 从拓扑「在任务与回执中查看」过来：这一条标成当前并滚到视野里。
  const focused = item.id === focusId;
  const article = useRef<HTMLElement>(null);
  useEffect(() => {
    if (focused) article.current?.scrollIntoView({ block: 'nearest' });
  }, [focused]);
  return (
    <li className="board-node">
      <article
        ref={article}
        className={cx('board-item', focused && 'board-item--current')}
        aria-label={item.title}
        aria-current={focused ? 'true' : undefined}
      >
        <header className="board-item__head">
          <StatusMark className="board-status" state={nodeMark(item, lease)}>
            {label}
          </StatusMark>
          <span className="board-item__depth">层级 {item.depth}</span>
        </header>
        <div className="board-item__body">
          <h3 className="board-item__title">{item.title}</h3>
          <code className="board-item__id">{item.id}</code>
          {item.parent ? (
            <p className="board-hint">
              父工作项 <code>{item.parent}</code>
            </p>
          ) : null}
          {item.creator ? (
            <p className="board-hint">
              创建实例：<code>{item.creator}</code>
            </p>
          ) : null}
          <p className="board-item__holder">
            {lease === 'current'
              ? '当前持有实例：'
              : lease === 'revoked'
                ? '等待停止的实例：'
                : lease === 'settling'
                  ? '写回中的实例：'
                  : item.holder
                    ? '最近持有实例：'
                    : '当前持有实例：'}
            <code>{item.holder ?? '无活跃认领'}</code>
          </p>
          {lease === 'expired' ? (
            <p className="board-warning">
              租约已到期，不再确认该实例当前持有工作项；等待服务器对账。
            </p>
          ) : null}
          {lease === 'revoked' ? (
            <p className="board-warning">
              已请求停止；工具调用已撤权，等待回执。
            </p>
          ) : null}
          {item.attempt ? (
            <dl className="board-attempt">
              <div>
                <dt>{item.attempt.active ? '本次执行' : '最近执行'}</dt>
                <dd>
                  <code>{item.attempt.id}</code>
                </dd>
              </div>
              <div>
                <dt>认领凭据（fence）</dt>
                <dd>{item.attempt.fence}</dd>
              </div>
              <div>
                <dt>{item.attempt.active ? '租约截止' : '最近租约截止'}</dt>
                <dd>
                  <time
                    dateTime={new Date(item.attempt.leaseUntilMs).toISOString()}
                  >
                    {new Date(item.attempt.leaseUntilMs).toLocaleString()}
                  </time>
                  {lease === 'ended' ? '（执行已结束）' : null}
                </dd>
              </div>
            </dl>
          ) : null}
          <p className="board-item__progress">
            {item.progress
              ? `${item.progress.tools} 次工具调用 · ${item.progress.edits} 次编辑`
              : '尚无进度回报'}
          </p>
          {item.progress ? (
            <p className="board-hint">
              最近进度{' '}
              <time
                dateTime={new Date(item.progress.lastProgressMs).toISOString()}
              >
                {new Date(item.progress.lastProgressMs).toLocaleString()}
              </time>
            </p>
          ) : null}
          {item.oldPolicy ? (
            <p className="board-warning">
              按旧版规则执行；当前撤权仍即时生效。
            </p>
          ) : null}
          <details className="board-details">
            <summary>独占路径 · {item.paths.length}</summary>
            {item.paths.length ? (
              <ul className="board-paths">
                {item.paths.map((p) => (
                  <li key={p}>
                    <code>{p}</code>
                  </li>
                ))}
              </ul>
            ) : (
              <p>只读工作项</p>
            )}
          </details>
          <details className="board-details">
            <summary>验收标准 · {item.criteria.length}</summary>
            <ol>
              {item.criteria.map((c) => (
                <li key={c}>{c}</li>
              ))}
            </ol>
            <p>
              规则版本 <code>{item.policyHash}</code>
            </p>
          </details>
          <CandidateDetails item={item} />
          {item.receipt ? <ReceiptDetails receipt={item.receipt} /> : null}
        </div>
      </article>
      {children.length ? (
        <ol className="board-children" aria-label={`${item.title}的子工作项`}>
          {children.map((child) => (
            <TaskNode
              key={child.item.id}
              node={child}
              now={now}
              focusId={focusId}
            />
          ))}
        </ol>
      ) : null}
    </li>
  );
}
function ReceiptDetails({
  receipt,
}: {
  receipt: NonNullable<BoardItem['receipt']>;
}) {
  return (
    <details className="board-receipt">
      <summary>收尾回执 · {receiptLabels[receipt.status]}</summary>
      <p>
        耗时 {Math.round(receipt.elapsedMs / 1000)} 秒 · {receipt.toolCalls}{' '}
        次工具调用
      </p>
      <h4>验收自评</h4>
      <ul>
        {receipt.acceptance.map((a) => (
          <li key={a.criterion}>
            {a.criterion}：{assessmentLabels[a.result]}
            {a.evidence.length ? (
              <ul>
                {a.evidence.map((e, i) => (
                  <li key={i}>
                    <code>{e.path}</code>
                    <small>
                      修订 {e.revision} · SHA-256 {e.sha256}
                    </small>
                  </li>
                ))}
              </ul>
            ) : null}
          </li>
        ))}
      </ul>
      <h4>产物</h4>
      {receipt.artifacts.length ? (
        <ul>
          {receipt.artifacts.map((p, i) => (
            <li key={i}>
              <code>{p.path}</code>
              <small>
                修订 {p.revision} · SHA-256 {p.sha256}
              </small>
            </li>
          ))}
        </ul>
      ) : (
        <p>回执未列产物。</p>
      )}
      {receipt.unlanded.length ? (
        <>
          <h4>未落地的补丁</h4>
          <ul>
            {receipt.unlanded.map((p, i) => (
              <li key={i}>
                <code>{p.path}</code>
                <small>SHA-256 {p.sha256}</small>
              </li>
            ))}
          </ul>
        </>
      ) : null}
      <h4>剩余事项</h4>
      {receipt.remaining.length ? (
        <ul>
          {receipt.remaining.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      ) : (
        <p>回执未列剩余事项。</p>
      )}
      <h4>回执经验</h4>
      {receipt.learned.length ? (
        <ul>
          {receipt.learned.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      ) : (
        <p>回执未列新经验。</p>
      )}
      <h4>收尾决定</h4>
      <p>
        需要 ADR：{receipt.closeout.adrRequired ? '是' : '否'} · 需要改规则：
        {receipt.closeout.ruleChangeRequired ? '是' : '否'}
      </p>
      {receipt.closeout.learned.length ? (
        <ul>
          {receipt.closeout.learned.map((p, i) => (
            <li key={i}>{p}</li>
          ))}
        </ul>
      ) : (
        <p>未记录新的经验。</p>
      )}
      {receipt.closeout.destinations.length ? (
        <p>经验去向：{receipt.closeout.destinations.join('、')}</p>
      ) : null}
    </details>
  );
}

function InstanceTree({ items }: { items: BoardItem[] }) {
  const nodes = new Map<
    string,
    { id: string; agentType: string; parent: string | null }
  >();
  for (const item of items)
    for (const chain of item.instanceLineages) {
      chain.forEach((node, index) =>
        nodes.set(node.id, {
          ...node,
          parent: index ? chain[index - 1].id : null,
        }),
      );
    }
  if (!nodes.size) return <p className="board-hint">实例关系字段暂不可用。</p>;
  function branch(parent: string | null): ReactNode {
    return (
      <ul className="board-instances">
        {[...nodes.values()]
          .filter((n) => n.parent === parent)
          .map((node) => {
            const held = items.filter(
              (item) =>
                item.holder === node.id &&
                boardLease(item, Date.now()) === 'current',
            );
            return (
              <li key={node.id}>
                {node.agentType} · <code>{node.id}</code> · 关联 {held.length}{' '}
                个活跃工作项
                {[...nodes.values()].some((n) => n.parent === node.id)
                  ? branch(node.id)
                  : null}
              </li>
            );
          })}
      </ul>
    );
  }
  return (
    <details className="board-details board-instance-tree">
      <summary>参与实例树 · {nodes.size}</summary>
      {branch(null)}
    </details>
  );
}
