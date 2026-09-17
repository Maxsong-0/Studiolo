import { useEffect, useRef, useState } from 'react';
import type { HistoryFile } from '../historyModel';
import { changedLines } from '../textComparison';

export function HistoryComparison({
  file,
  draft,
  revision,
  allowed,
  onRestore,
}: {
  file: HistoryFile;
  draft: string;
  revision: string;
  allowed: boolean;
  onRestore: () => void;
}) {
  const [confirming, setConfirming] = useState(false);
  const trigger = useRef<HTMLButtonElement>(null);
  const confirmation = useRef<HTMLElement>(null);
  useEffect(() => {
    if (confirming) confirmation.current?.focus();
  }, [confirming]);
  const cancel = () => {
    setConfirming(false);
    trigger.current?.focus();
  };
  const changes = changedLines(draft, file.content);
  return (
    <section className="history-comparison" aria-label="历史版本比较">
      <p>
        当前编辑基线：<code>{revision}</code>
        。左侧包含尚未保存的编辑；右侧为所选历史原文。
      </p>
      <div className="history-versions">
        <section>
          <h4>当前编辑内容</h4>
          <pre>{draft}</pre>
        </section>
        <section>
          <h4>所选历史内容</h4>
          <pre>
            {changes
              ? changes.map((line, index) => (
                  <span key={index} className="comparison-line">
                    {line.changed ? (
                      <mark>{line.text || '\u00a0'}</mark>
                    ) : (
                      line.text || '\u00a0'
                    )}
                  </span>
                ))
              : file.content}
          </pre>
        </section>
      </div>
      <p>差异仅标记共同首尾之间的变化，不自动合并。</p>
      {!allowed ? (
        <p>
          请先处理草稿、在途请求及编辑权限；当前版本与所选版本相同也无需恢复。
        </p>
      ) : null}
      <button
        ref={trigger}
        className="outline-button"
        disabled={!allowed}
        aria-expanded={confirming && allowed}
        onClick={() => setConfirming(true)}
      >
        恢复此历史版本…
      </button>
      {confirming && allowed ? (
        <section
          ref={confirmation}
          tabIndex={-1}
          role="alertdialog"
          aria-label="确认恢复历史版本"
          className="editor-leave"
          onKeyDown={(event) => {
            if (event.key === 'Escape') {
              event.preventDefault();
              event.stopPropagation();
              cancel();
            }
          }}
        >
          <h4>确认恢复到所选历史版本？</h4>
          <p>
            只恢复当前已有页面，以当前核验版本作为冲突检查条件。必须取得恢复审计回执，随后还会另行核对磁盘。
          </p>
          <button
            className="outline-button"
            onClick={() => {
              setConfirming(false);
              onRestore();
            }}
          >
            确认恢复此版本
          </button>
          <button className="text-button" onClick={cancel}>
            取消恢复
          </button>
        </section>
      ) : null}
    </section>
  );
}
