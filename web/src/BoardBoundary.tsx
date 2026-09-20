import { useEffect, useState } from 'react';
import type { ReactNode } from 'react';
import { CandidateDetails } from './ExecutionCandidates';
import { ProjectTimeline } from './ProjectTimeline';
import { LocalDispatch } from './LocalDispatch';
import { ApiError, getJson } from './sessionHttp';
import { boardItems, boardLease, boardTree } from './boardModel';
import type { BoardItem, BoardNode, BoardStatus } from './boardModel';
import './styles/board.css';

type State =
  | { kind: 'loading' }
  | { kind: 'ready'; items: BoardItem[]; at: number }
  | { kind: 'error'; message: string };
const REFRESH_MS = 15_000;
const REQUEST_MS = 10_000;
const STALE_MS = 30_000;
const labels: Record<BoardStatus, string> = {
  draft: '草稿，尚未排队',
  queued: '待认领',
  running: '进行中',
  claimed: '已认领',
  committing: '写回中',
  cancel_requested: '等待停止回执',
  succeeded: '已完成',
  failed: '失败',
  cancelled: '已停止',
  abandoned: '已放弃',
  needs_review: '待审阅',
  needs_revision: '等待修订',
  needs_reconfirm: '等待重新确认',
  approved: '已审阅通过',
};
const receiptLabels = {
  completed: '已完成',
  failed: '失败',
  stopped: '已停止',
  abandoned: '已放弃',
};
const assessmentLabels = { pass: '通过', fail: '未通过', unverified: '未验证' };
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
  const [selected, setSelected] = useState('');
  const [replay, setReplay] = useState(false);
  const project = projects.some((p) => p.root === selected)
    ? selected
    : (projects[0]?.root ?? '');
  return (
    <section className="board-panel" aria-label="协作黑板">
      <div className="board-toolbar">
        <label>
          项目{' '}
          <select
            value={project}
            onChange={(event) => setSelected(event.target.value)}
          >
            {projects.map((p) => (
              <option key={p.root} value={p.root}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <span>状态核对 · 每 15 秒刷新</span>
        <button onClick={() => setReplay((v) => !v)}>
          {replay ? '返回实时黑板' : '项目时间回放'}
        </button>
      </div>
      {project && <LocalDispatch key={project} project={project} />}
      {project ? (
        replay ? (
          <ProjectTimeline key={project} project={project} />
        ) : (
          <BoardProject key={project} project={project} />
        )
      ) : (
        <p>当前没有可查看的项目。</p>
      )}
    </section>
  );
}
function BoardProject({ project }: { project: string }) {
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
      }, REQUEST_MS);
      deadline = timeout;
      try {
        const value = await getJson<unknown>(
          `/api/v1/board?project=${encodeURIComponent(project)}`,
          request.signal,
        );
        if (disposed || expired || request.signal.aborted) return;
        const items = boardItems(value, project);
        setState({ kind: 'ready', items, at: Date.now() });
      } catch (error) {
        if (disposed || expired || request.signal.aborted) return;
        setState({
          kind: 'error',
          message:
            error instanceof ApiError && error.status === 404
              ? '此工作台尚未启用协作黑板，或黑板入口已失效。'
              : error instanceof Error
                ? error.message
                : '黑板连接不可用。',
        });
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
  if (state.kind === 'loading') return <p role="status">正在读取协作状态…</p>;
  if (state.kind === 'error')
    return <Unavailable message={state.message} retry={retry} />;
  return <BoardSnapshot items={state.items} at={state.at} retry={retry} />;
}
function Unavailable({
  message,
  retry,
}: {
  message: string;
  retry: () => void;
}) {
  return (
    <div className="board-unavailable" role="alert">
      <h3>实时状态不可确认</h3>
      <p>{message}</p>
      <p>暂停依据旧状态进行认领和写入。</p>
      <button onClick={retry}>重新核对</button>
    </div>
  );
}
function BoardSnapshot({
  items,
  at,
  retry,
}: {
  items: BoardItem[];
  at: number;
  retry: () => void;
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
      <Unavailable
        message="上次核对已超过 30 秒，当前连接状态不可确认。"
        retry={retry}
      />
    );
  const active = items.filter(
    (item) => boardLease(item, now) === 'current',
  ).length;
  return (
    <>
      <p className="board-freshness">
        已核对{' '}
        <time dateTime={new Date(at).toISOString()}>
          {new Date(at).toLocaleTimeString()}
        </time>{' '}
        · {items.length} 个工作项 · {active} 项租约有效
      </p>
      <p className="board-hint">
        租约时间按本机时钟显示；认领与写入以服务器当次裁决为准。
      </p>
      <InstanceTree items={items} />
      {items.length ? (
        <ol className="board-tree" aria-label="工作项树">
          {boardTree(items).map((node) => (
            <TaskNode key={node.item.id} node={node} now={now} />
          ))}
        </ol>
      ) : (
        <div className="board-empty">
          <h3>无活动</h3>
          <p>此项目当前没有黑板工作项。</p>
        </div>
      )}
    </>
  );
}
function TaskNode({ node, now }: { node: BoardNode; now: number }) {
  const { item, children } = node;
  const lease = boardLease(item, now);
  const blocked = children.some((child) => !terminal.has(child.item.status));
  const label =
    item.status === 'queued' && blocked
      ? '等待子项收尾'
      : lease === 'expired'
        ? '租约已到期，状态待核对'
        : labels[item.status];
  return (
    <li>
      <article className="board-item" aria-label={item.title}>
        <header>
          <span className="board-status">{label}</span>
          <span>层级 {item.depth}</span>
        </header>
        <h3>{item.title}</h3>
        <code>{item.id}</code>
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
        <p>
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
        <p>
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
          <p className="board-warning">按旧版规则执行；当前撤权仍即时生效。</p>
        ) : null}
        <details>
          <summary>独占路径 · {item.paths.length}</summary>
          {item.paths.length ? (
            <ul>
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
        <details>
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
      </article>
      {children.length ? (
        <ol className="board-children" aria-label={`${item.title}的子工作项`}>
          {children.map((child) => (
            <TaskNode key={child.item.id} node={child} now={now} />
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
      <ul>
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
    <details className="board-item">
      <summary>参与实例树 · {nodes.size}</summary>
      {branch(null)}
    </details>
  );
}
