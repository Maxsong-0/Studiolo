import type {
  DirectoryDrift,
  ProjectOperationsData,
} from '../projectOperations';
import type { OpenSource } from '../types';
import {
  Button,
  Cell,
  CellHeader,
  GridFrame,
  StatusMark,
  type StatusState,
} from '../ui';
import '../styles/operations.css';

type DriftStatus = DirectoryDrift['status'];

const statusLabels: Record<DriftStatus, string> = {
  present: '已声明 · 目录存在',
  missing: '已声明 · 目录缺失',
  undocumented: '目录存在 · 未声明',
  unverified: '未核验',
};

/** 红色只给「声明有但目录没有」「目录有但声明没有」；存在是实心墨块，未核验是休眠框。 */
const statusMarks: Record<DriftStatus, StatusState> = {
  present: 'ok',
  missing: 'alert',
  undocumented: 'alert',
  unverified: 'dormant',
};

/** 计数的短标签：不与逐条状态的原文重名。 */
const tallyLabels: [DriftStatus, string][] = [
  ['present', '存在'],
  ['missing', '缺失'],
  ['undocumented', '未声明'],
  ['unverified', '未核验'],
];

/**
 * 双向目录漂移：左 4 列比较范围、仓库观测与计数，右 8 列逐条比对结果。
 * 仓库未就绪时每条一律按「未核验」显示，计数随之按显示的状态计。
 */
export function OperationDrift({
  data,
  onOpen,
}: {
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const observed = data.repository.observed_at;
  const verified = data.repository.status === 'ready';
  const status =
    data.repository.status === 'unbound'
      ? '仓库未绑定 · 未核验'
      : verified
        ? '本次目录列举完成'
        : '仓库不可用 · 未核验';
  const shown = data.drift.map((item) =>
    verified ? item.status : ('unverified' as const),
  );
  const tally = (status: DriftStatus) =>
    shown.filter((item) => item === status).length;
  return (
    <Cell
      span={12}
      flush
      as="section"
      className="operation-block"
      aria-labelledby="operation-drift-title"
    >
      <GridFrame className="operation-subgrid">
        <Cell span={12}>
          <CellHeader
            title={<span id="operation-drift-title">双向目录漂移</span>}
            meta={status}
          />
          <p className="operation-note">
            {data.repository.status === 'unbound'
              ? '尚未配置此项目的本地仓库绑定。以下声明没有经过实际目录核验。'
              : '仅比较下列父目录的直属目录；没有观测证据的声明保持未核验。此结果不代表全仓库检查通过。'}
          </p>
        </Cell>
        <Cell span={4} className="operation-facts">
          <div className="operation-fact">
            <strong>比较父目录</strong>
            {data.comparison_parents.length ? (
              <ul className="operation-scope-list">
                {data.comparison_parents.map((path) => (
                  <li key={path}>
                    <code>{path || '.'}</code>
                  </li>
                ))}
              </ul>
            ) : (
              <span>暂无可比较的父目录</span>
            )}
          </div>
          <p className="operation-fact">
            <strong>仓库观测</strong>
            {observed !== null ? (
              <time dateTime={new Date(Number(observed) * 1000).toISOString()}>
                {new Date(Number(observed) * 1000).toLocaleString()}
              </time>
            ) : (
              '暂无可信观测时间'
            )}
            <span>
              仓库目录按本次读取独立观测；文档声明绑定快照 {data.generation}。
            </span>
          </p>
          {data.drift.length ? (
            <dl className="operation-tally">
              {tallyLabels.map(([key, label]) => {
                const count = tally(key);
                const signal = statusMarks[key] === 'alert' && count > 0;
                return (
                  <div key={key}>
                    <dt>{label}</dt>
                    <dd className={signal ? 'is-signal' : undefined}>
                      {signal ? (
                        <span className="ui-signal-mark" aria-hidden="true" />
                      ) : null}
                      {count}
                    </dd>
                  </div>
                );
              })}
            </dl>
          ) : null}
        </Cell>
        <Cell span={8}>
          {data.drift.length ? (
            <>
              <div className="operation-drift-head" aria-hidden="true">
                <span>目录</span>
                <span>核验状态</span>
                <span>声明来源</span>
              </div>
              <ul className="operation-drift-list" aria-label="目录比对结果">
                {data.drift.map((item, index) => {
                  const currentStatus = shown[index];
                  return (
                    <li key={`${item.path}:${index}`}>
                      <code>{item.path}</code>
                      <StatusMark state={statusMarks[currentStatus]}>
                        {statusLabels[currentStatus]}
                      </StatusMark>
                      {item.source ? (
                        <Button
                          variant="text"
                          size="sm"
                          onClick={() => onOpen(item.source!)}
                          aria-label={`${item.path}，查看目录声明来源`}
                        >
                          查看声明
                        </Button>
                      ) : (
                        <span className="operation-muted">无声明来源</span>
                      )}
                    </li>
                  );
                })}
              </ul>
            </>
          ) : (
            <p className="operation-muted">
              暂无目录比对结果。请先核对结构声明、仓库绑定与比较范围。
            </p>
          )}
        </Cell>
      </GridFrame>
    </Cell>
  );
}
