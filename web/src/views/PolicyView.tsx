import { PolicyBoundary } from '../PolicyBoundary';

/**
 * 策略（设计规范第 3、9 节）：与驾驶舱、协作同一版式——图名只给读屏（顶栏当前导航项已写明「策略」），
 * 下面直接是策略平面的 12 列网格（GridFrame／Cell／CellHeader，见 PolicyBoundary 与 PolicyPanel）。
 * 不依赖工作台快照，快照读不出来时也要能发布或撤销接入。
 */
export function PolicyView() {
  return (
    <div className="view view-policy">
      <h1 className="sr-only">策略</h1>
      <PolicyBoundary />
    </div>
  );
}
