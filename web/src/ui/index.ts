/*
 * T-130 基础组件库。规范：docs/design/t130-design-system.md。
 * 组件只引用 tokens.css 的新令牌；样式全部在 ui.css（CSP 只许 style-src 'self'）。
 */
import './ui.css';

export { BigNumber, type BigNumberProps } from './BigNumber';
export {
  Button,
  type ButtonProps,
  type ButtonSize,
  type ButtonVariant,
} from './Button';
export { CircledNumber, type CircledNumberProps } from './CircledNumber';
export {
  Cell,
  CellHeader,
  GridFrame,
  type CellElement,
  type CellHeaderProps,
  type CellProps,
  type GridFrameProps,
} from './Grid';
export { KeyHint, type KeyHintProps } from './KeyHint';
export { Lanes, type Lane, type LanesProps } from './Lanes';
export type { LaneBlock, LaneTone } from './geometry';
export { Sparkbars, type SparkbarsProps } from './Sparkbars';
export { Stat, type StatProps, type StatTone } from './Stat';
export {
  EmptyState,
  ErrorState,
  Skeleton,
  type EmptyStateProps,
  type ErrorStateProps,
  type SkeletonProps,
} from './States';
export {
  StatusMark,
  type StatusMarkProps,
  type StatusState,
} from './StatusMark';
export { Surface, type SurfaceElement, type SurfaceProps } from './Surface';
export { Tabs, type TabItem, type TabsProps } from './Tabs';
export {
  TitleBlock,
  type TitleBlockCell,
  type TitleBlockProps,
} from './TitleBlock';
