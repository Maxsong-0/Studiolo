import type { MarkTone } from './markTone';
import './brand.css';

/**
 * 标志「纸与影」（设计规范第 14 节）。边长分 8 格，坐标按 48 等分（1 格 = 6）：
 * 纸是左上角 6×6 格的方块；影是贴着右边和下边、1 格宽的折角，离纸 1 格，从第 2 格起。
 *
 * - 颜色走令牌（brand.css）：纸是墨色；影在 standard 下是信号红，mono 下和纸同色。
 * - size 是边长的像素数，只取 8 的倍数，每格才落在整数像素上；最小 16。
 * - 给了 label 就是一张有名字的图；不给就只是装饰、读屏跳过（旁边已有文字「Studiolo」时用）。
 */
export function BrandMark({
  tone,
  size,
  label,
}: {
  tone: MarkTone;
  size: 16 | 24 | 32 | 48;
  label?: string;
}) {
  return (
    <svg
      className="brand-mark"
      data-tone={tone}
      width={size}
      height={size}
      viewBox="0 0 48 48"
      shapeRendering="crispEdges"
      role={label ? 'img' : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <rect className="brand-mark-paper" width="36" height="36" />
      <path className="brand-mark-shadow" d="M42 12H48V48H12V42H42Z" />
    </svg>
  );
}
