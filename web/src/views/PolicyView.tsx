import { PolicyBoundary } from '../PolicyBoundary';
import { ViewHeader } from './ViewHeader';

/** 策略（A 轮薄包装）：不依赖工作台快照，快照读不出来时也要能发布或吊销。 */
export function PolicyView() {
  return (
    <div className="view view-policy">
      <ViewHeader
        title="策略"
        description="规则母本、skills 与各 AI 能力集：提交 desired、校验 diff、经 One-time Code 发布，三级吊销。"
      />
      <PolicyBoundary />
    </div>
  );
}
