import { DETAIL_LIMIT, entryDetail } from './detail';
import type { PendingEntry } from './pending';

/**
 * 选中行就地展开的细节（T-197；设计规范第 6 节）：每段一个等宽小标签带条数，下面一条一行。
 * 只读，不带任何动作；内容怎么取、每段列几条见 detail.ts。没有可列的就什么都不画。
 * 这里不用红：驾驶舱的红只给巨号和「停下等你」，扩大项用实心方块加「扩大」两个字。
 * 就地批准的执行预览开着时（`full`，T-200）每段全部列出，不截到前几条。
 */
export function EntryDetail({
  entry,
  full = false,
}: {
  entry: PendingEntry;
  /** 全部列出，不截条数。 */
  full?: boolean;
}) {
  const sections = entryDetail(entry, full ? Infinity : DETAIL_LIMIT);
  if (sections.length === 0) return null;
  return (
    <div className="cockpit-detail">
      {sections.map((section) => (
        <section key={section.id} className="cockpit-detail__section">
          <h3 className="cockpit-detail__label">
            {section.label} <span>{section.count}</span>
          </h3>
          <ul className="cockpit-detail__lines">
            {section.lines.map((line, index) => (
              <li key={index} className="cockpit-detail__line">
                {line.mark ? (
                  <span className="cockpit-detail__mark">
                    <span aria-hidden="true">{line.mark.glyph}</span>
                    <span className="sr-only">{line.mark.name}</span>
                  </span>
                ) : null}
                <span className="cockpit-detail__body">
                  {line.code !== null ? <code>{line.code}</code> : null}
                  {line.parts.map((part, at) => (
                    <span key={at}>{part}</span>
                  ))}
                  {line.note !== null ? (
                    <span className="cockpit-detail__note">{line.note}</span>
                  ) : null}
                </span>
              </li>
            ))}
          </ul>
          {section.more > 0 || section.note !== null ? (
            <p className="cockpit-detail__more">
              {section.more > 0 ? `还有 ${section.more} 条没列出来。` : null}
              {section.note}
            </p>
          ) : null}
        </section>
      ))}
    </div>
  );
}
