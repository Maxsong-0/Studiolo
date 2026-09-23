import type { EditorSnapshot } from '../EditorController';
import { changedLines } from '../textComparison';
import { Cell, GridFrame } from '../ui';
import '../styles/editor.css';

function ComparedText({ base, current }: { base: string; current: string }) {
  const lines = changedLines(base, current);
  if (!lines)
    return (
      <>
        <p>行数较多，显示纯文本供核对。</p>
        <pre>{current}</pre>
      </>
    );
  return (
    <pre>
      {lines.map((line, index) => (
        <span className="comparison-line" key={index}>
          {line.changed ? <mark>{line.text || ' '}</mark> : line.text || ' '}
        </span>
      ))}
    </pre>
  );
}

/**
 * 三份原文等宽对照：编辑基线通栏在上，本页草稿与当前磁盘版本两栏并排（窄屏收成单列）。
 * 变更行都相对基线标出，顺序与改版前一致：基线、草稿、磁盘。
 */
export function EditorComparison({ state }: { state: EditorSnapshot }) {
  return (
    <section className="editor-comparison" aria-label="版本差异比较">
      <h3>比较基线、草稿与当前版本</h3>
      <p>
        标出共同首尾之外的变更区。当前版本来自独立读取，可能继续变化；继续保存仍会核对版本。
      </p>
      <GridFrame columns={2} className="editor-versions">
        <Cell as="section" span={2} className="editor-version">
          <h4>编辑基线</h4>
          <pre>{state.base?.content}</pre>
        </Cell>
        <Cell as="section" span={1} className="editor-version">
          <h4>本页草稿</h4>
          <ComparedText
            base={state.base?.content ?? ''}
            current={state.draft}
          />
        </Cell>
        <Cell as="section" span={1} className="editor-version">
          <h4>当前磁盘版本</h4>
          {state.remote ? (
            <ComparedText
              base={state.base?.content ?? ''}
              current={state.remote.content}
            />
          ) : (
            <p>当前内容尚未取得，不能建立新的保存基线。</p>
          )}
        </Cell>
      </GridFrame>
    </section>
  );
}
