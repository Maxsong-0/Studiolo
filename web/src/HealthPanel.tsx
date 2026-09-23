import type { HealthReport, HealthState } from './healthApi';
import { StatusMark } from './ui';
import type { StatusState } from './ui';
import './styles/health.css';

const HEALTH_LABEL: Record<HealthState, string> = {
  normal: '正常',
  degraded: '降级',
  unknown: '未知',
};

/** 整体状态的方块：正常墨色实心、降级红色实心、未知休眠虚线——未知绝不画成正常。 */
const MARK: Record<HealthState, StatusState> = {
  normal: 'ok',
  degraded: 'alert',
  unknown: 'dormant',
};

/**
 * 整体状态一行。「整体状态」后面紧跟的元素带 data-state（健康三态原值），
 * 方块与颜色只是加强，状态字本身写着正常／降级／未知。
 */
export function HealthOverall({ state }: { state: HealthState }) {
  return (
    <p className="health-overall">
      <span className="health-overall-label">整体状态</span>
      <strong className="health-overall-state" data-state={state}>
        <StatusMark state={MARK[state]}>{HEALTH_LABEL[state]}</StatusMark>
      </strong>
    </p>
  );
}

/**
 * 只读展示：整体状态一行＋检查项表（项目、状态、详情）。任何非 normal 状态都不使用"绿"的语义类名。
 * 检查项仍是列表（每项一个标题），表头只给眼睛看；窄屏收成上下排。
 */
export function HealthPanel({ report }: { report: HealthReport }) {
  return (
    <section className="health-panel" aria-label="本机健康">
      <HealthOverall state={report.overall} />
      {report.evidenceComplete ? null : (
        <p className="health-incomplete">
          证据不完整，缺失项一律显示未知，不代表一切正常。
        </p>
      )}
      {report.sections.length ? (
        <div className="health-table-head" aria-hidden="true">
          <span>项目</span>
          <span>状态</span>
          <span>详情</span>
        </div>
      ) : null}
      <ul className="health-sections" aria-label="健康检查项">
        {report.sections.map((section) => (
          <li key={section.code} className="health-row">
            {/* 浮层里：h2「工作台状态」→ h3「本机健康」→ h4 各检查项。 */}
            <h4>{section.title}</h4>
            <p className="health-state" data-state={section.state}>
              {HEALTH_LABEL[section.state]}
            </p>
            <p className="health-detail">{section.detail}</p>
          </li>
        ))}
      </ul>
    </section>
  );
}
