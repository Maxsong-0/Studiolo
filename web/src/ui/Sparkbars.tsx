import { cx } from './dom';
import { sparkbarsGeometry } from './geometry';
import './ui.css';

export interface SparkbarsProps {
  values: readonly number[];
  /** 限值：画一条红色虚线。 */
  limit?: number | null;
  /** 纵轴上限，缺省取数据与限值中的最大者。 */
  max?: number | null;
  /** 图高（px），缺省 64。 */
  height?: number;
  /** 读屏读到的完整说法，如「最近 24 轮同步耗时，限值 120 秒」。 */
  label: string;
  className?: string;
}

/**
 * 细竖条柱图。横向随容器伸缩，柱宽与虚线用 non-scaling-stroke 保持固定像素，
 * 几何全部写在 SVG 属性里，颜色与线型在 ui.css。
 */
export function Sparkbars({
  values,
  limit,
  max,
  height = 64,
  label,
  className,
}: SparkbarsProps) {
  const h = Number.isFinite(height) && height > 0 ? height : 64;
  const geometry = sparkbarsGeometry(values, h, limit, max);
  return (
    <svg
      className={cx('ui-sparkbars', className)}
      role="img"
      aria-label={label}
      viewBox={`0 0 ${geometry.width} ${h}`}
      preserveAspectRatio="none"
      height={h}
    >
      {geometry.bars.map((bar, index) =>
        bar.top < h ? (
          <line
            key={index}
            className="ui-sparkbars__bar"
            x1={bar.x}
            x2={bar.x}
            y1={h}
            y2={bar.top}
            vectorEffect="non-scaling-stroke"
          />
        ) : null,
      )}
      {geometry.limitY !== null && (
        <line
          className="ui-sparkbars__limit"
          x1={0}
          x2={geometry.width}
          y1={geometry.limitY}
          y2={geometry.limitY}
          vectorEffect="non-scaling-stroke"
        />
      )}
    </svg>
  );
}
