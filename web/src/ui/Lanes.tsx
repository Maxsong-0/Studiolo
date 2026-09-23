import { Fragment } from 'react';
import { cx } from './dom';
import { laneBoxes, type LaneBlock, type LaneBox } from './geometry';
import './ui.css';

export interface Lane {
  name: string;
  blocks: readonly LaneBlock[];
}

export interface LanesProps {
  /** 时间范围起点，与 blocks 的 start/end 同一单位（小时或毫秒时间戳都行）。 */
  from: number;
  to: number;
  lanes: readonly Lane[];
  /** 轨道下方的刻度文字，首尾对齐 from/to，其余均分。 */
  ticks?: readonly string[];
  /** 读屏读到的说法；不给时按泳道自动生成摘要。 */
  label?: string;
  className?: string;
}

function summary(
  lanes: readonly { name: string; boxes: readonly LaneBox[] }[],
  ticks: readonly string[] | undefined,
): string {
  const range =
    ticks && ticks.length > 1 ? `${ticks[0]}–${ticks[ticks.length - 1]}` : '';
  const parts = lanes.map(({ name, boxes }) => {
    const work = boxes.filter((box) => box.tone === 'work').length;
    const owner = boxes.length - work;
    const counts = [
      work > 0 ? `工作 ${work} 段` : '',
      owner > 0 ? `审批 ${owner} 段` : '',
    ].filter(Boolean);
    return `${name}：${counts.length > 0 ? counts.join('，') : '无'}`;
  });
  return [`时间泳道${range ? ` ${range}` : ''}`, ...parts].join('；');
}

/**
 * 时间泳道：一条执行者一条轨道，黑块＝工作，红块＝你的审批。
 * 块裁剪到 [from, to]，最窄 0.6%；整张图是一幅图（role=img），文字摘要进 aria-label。
 */
export function Lanes({
  from,
  to,
  lanes,
  ticks,
  label,
  className,
}: LanesProps) {
  const laid = lanes.map((lane) => ({
    name: lane.name,
    boxes: laneBoxes(from, to, lane.blocks),
  }));
  return (
    <div
      className={cx('ui-lanes', className)}
      role="img"
      aria-label={label ?? summary(laid, ticks)}
    >
      {laid.map((lane, laneIndex) => (
        <Fragment key={`${laneIndex}-${lane.name}`}>
          <span className="ui-lanes__name">{lane.name}</span>
          <span className="ui-lanes__track">
            <svg
              className="ui-lanes__svg"
              viewBox="0 0 100 10"
              preserveAspectRatio="none"
            >
              {lane.boxes.map((box, index) => (
                <rect
                  key={index}
                  className={`ui-lanes__block ui-lanes__block--${box.tone}`}
                  x={box.left}
                  width={box.width}
                  y={0}
                  height={10}
                />
              ))}
            </svg>
          </span>
        </Fragment>
      ))}
      {ticks && ticks.length > 0 && (
        <>
          <span className="ui-lanes__name" />
          <span className="ui-lanes__ticks">
            {ticks.map((tick, index) => (
              <span key={`${index}-${tick}`}>{tick}</span>
            ))}
          </span>
        </>
      )}
    </div>
  );
}
