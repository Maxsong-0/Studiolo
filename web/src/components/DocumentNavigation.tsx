import type {
  DocumentNavigationData,
  OpenSource,
  OutgoingLink,
} from '../types';
import { Icon } from './Icon';
import '../styles/navigation.css';

const unresolvedLabels = {
  ambiguous: '存在同名文档，请选择完整路径',
  missing: '未找到目标文档',
  missing_heading: '未找到目标标题',
  ambiguous_heading: '存在同名标题，请选择位置',
} as const;

function OutgoingEntry({
  link,
  onNavigate,
}: {
  link: OutgoingLink;
  onNavigate: OpenSource;
}) {
  return (
    <li>
      <div className="navigation-link-label">{link.label || link.raw}</div>
      <span className="navigation-context">
        {link.raw} · 原文第 {link.source.line} 行
      </span>
      {link.status === 'resolved' ? (
        <button
          className="navigation-target"
          onClick={() => onNavigate(link.targets[0])}
        >
          {link.targets[0].path} · 第 {link.targets[0].line} 行
        </button>
      ) : (
        <>
          <span className="navigation-unresolved">
            {(link.status === 'ambiguous' ||
              link.status === 'ambiguous_heading') &&
            !link.targets.length
              ? '暂无可定位候选，请核对原文或导航截断提示。'
              : unresolvedLabels[link.status]}
          </span>
          {(link.status === 'ambiguous' ||
            link.status === 'ambiguous_heading') &&
          link.targets.length > 0 ? (
            <ul
              className="navigation-candidates"
              aria-label={`${link.label || link.raw} 的候选目标`}
            >
              {link.targets.map((target) => (
                <li key={`${target.path}:${target.line}`}>
                  <button
                    className="navigation-target"
                    onClick={() => onNavigate(target)}
                  >
                    {target.path} · 第 {target.line} 行
                    <span className="navigation-context">{target.title}</span>
                  </button>
                </li>
              ))}
            </ul>
          ) : null}
        </>
      )}
    </li>
  );
}

/**
 * 阅读器左栏（256px）：大纲、本页链接、反链三段各自可折叠，大纲默认展开；
 * 当前位置所属的标题黑底反白（aria-current=location）。
 */
export function DocumentNavigation({
  navigation,
  onNavigate,
  currentLine,
}: {
  navigation: DocumentNavigationData;
  onNavigate: OpenSource;
  currentLine?: number;
}) {
  const currentHeading = navigation.outline.reduce(
    (current, entry, index) =>
      currentLine !== undefined && entry.source.line <= currentLine
        ? index
        : current,
    -1,
  );
  return (
    <nav className="document-navigation" aria-label="原文导航">
      {navigation.truncated ? (
        <p className="navigation-limit" role="status">
          导航条目较多，当前仅显示部分结果。
        </p>
      ) : null}
      <details open>
        <summary>
          <span>大纲（{navigation.outline.length}）</span>
          <Icon name="chevron" />
        </summary>
        {navigation.outline.length ? (
          <ol className="navigation-list">
            {navigation.outline.map((entry, index) => (
              <li
                key={`${entry.source.line}:${index}`}
                className={`outline-level-${entry.level}`}
              >
                <button
                  className="navigation-target"
                  aria-current={
                    index === currentHeading ? 'location' : undefined
                  }
                  onClick={() => onNavigate(entry.source)}
                >
                  {entry.title}
                  <span className="navigation-context">
                    第 {entry.source.line} 行 · {entry.level} 级标题
                  </span>
                </button>
              </li>
            ))}
          </ol>
        ) : (
          <p className="navigation-empty">本页没有可跳转的标题。</p>
        )}
      </details>
      <details>
        <summary>
          <span>本页链接（{navigation.outgoing.length}）</span>
          <Icon name="chevron" />
        </summary>
        {navigation.outgoing.length ? (
          <ul className="navigation-list">
            {navigation.outgoing.map((link, index) => (
              <OutgoingEntry
                key={`${link.source.line}:${index}`}
                link={link}
                onNavigate={onNavigate}
              />
            ))}
          </ul>
        ) : (
          <p className="navigation-empty">本页没有已识别的笔记链接。</p>
        )}
      </details>
      <details>
        <summary>
          <span>反链（{navigation.backlinks.length}）</span>
          <Icon name="chevron" />
        </summary>
        {navigation.backlinks.length ? (
          <ul className="navigation-list">
            {navigation.backlinks.map((entry, index) => (
              <li key={`${entry.source.path}:${entry.source.line}:${index}`}>
                <button
                  className="navigation-target"
                  onClick={() => onNavigate(entry.source)}
                >
                  {entry.title}
                  <span className="navigation-context">
                    {entry.source.path} · 第 {entry.source.line} 行
                  </span>
                </button>
                <p className="navigation-context">{entry.context}</p>
              </li>
            ))}
          </ul>
        ) : (
          <p className="navigation-empty">当前快照没有指向本页的反链。</p>
        )}
      </details>
    </nav>
  );
}
