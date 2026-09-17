import type { EditorSnapshot } from '../EditorController';
import { changedLines } from '../textComparison';

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

export function EditorComparison({ state }: { state: EditorSnapshot }) {
  return (
    <section className="editor-comparison" aria-label="版本差异比较">
      <h3>比较基线、草稿与当前版本</h3>
      <p>
        标出共同首尾之外的变更区。当前版本来自独立读取，可能继续变化；继续保存仍会核对版本。
      </p>
      <div className="editor-versions">
        <section>
          <h4>编辑基线</h4>
          <pre>{state.base?.content}</pre>
        </section>
        <section>
          <h4>本页草稿</h4>
          <ComparedText
            base={state.base?.content ?? ''}
            current={state.draft}
          />
        </section>
        <section>
          <h4>当前磁盘版本</h4>
          {state.remote ? (
            <ComparedText
              base={state.base?.content ?? ''}
              current={state.remote.content}
            />
          ) : (
            <p>当前内容尚未取得，不能建立新的保存基线。</p>
          )}
        </section>
      </div>
    </section>
  );
}
