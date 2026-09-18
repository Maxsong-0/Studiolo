import type { HealthReport, HealthState } from './healthApi';

const LABEL: Record<HealthState, string> = {
  normal: '正常',
  degraded: '降级',
  unknown: '未知',
};

/** 只读展示。任何非 normal 状态都不使用"绿"的语义类名。 */
export function HealthPanel({ report }: { report: HealthReport }) {
  return (
    <section className="health-panel" aria-label="本机健康">
      <p className={`health-overall health-${report.overall}`}>
        <span>整体状态</span>
        <strong data-state={report.overall}>{LABEL[report.overall]}</strong>
      </p>
      {report.evidenceComplete ? null : (
        <p className="health-incomplete">
          证据不完整，缺失项一律显示未知，不代表一切正常。
        </p>
      )}
      <ul className="health-sections" aria-label="健康检查项">
        {report.sections.map((section) => (
          <li key={section.code} className={`health-${section.state}`}>
            <h3>{section.title}</h3>
            <p className="health-state" data-state={section.state}>
              {LABEL[section.state]}
            </p>
            <p className="health-detail">{section.detail}</p>
          </li>
        ))}
      </ul>
    </section>
  );
}
