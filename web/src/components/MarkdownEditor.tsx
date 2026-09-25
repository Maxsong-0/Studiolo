import { useEffect, useRef, useState } from 'react';
import { Compartment, EditorState, Transaction } from '@codemirror/state';
import { EditorView, keymap } from '@codemirror/view';
import { defaultKeymap, history, historyKeymap } from '@codemirror/commands';
import { markdown } from '@codemirror/lang-markdown';
import '../styles/editor.css';

interface Props {
  value: string;
  readOnly: boolean;
  onChange: (value: string) => void;
  canChange: (value: string) => boolean;
  onLimit: () => void;
  onBlur: () => void;
  onComposition: (active: boolean) => void;
}

/** 一个 CodeMirror 状态拥有全部编辑、撤销和 IME 行为，React 不重建输入中的文档。 */
export function MarkdownEditor(props: Props) {
  const mount = useRef<HTMLDivElement>(null);
  const instance = useRef<EditorView | null>(null);
  const current = useRef(props);
  const [failure, setFailure] = useState('');
  const [editable] = useState(() => new Compartment());
  // 刚按在「不保存区」（[data-editor-no-save]）里：Safari、macOS 上的 Firefox 与触屏点按按钮时
  // 不把焦点交给按钮，失焦事件的 relatedTarget 为 null，只看它会把这类点按当成离开编辑区而自动保存。
  const noSavePress = useRef(false);
  useEffect(() => {
    current.current = props;
  });
  useEffect(() => {
    const doc = mount.current?.ownerDocument ?? document;
    const view = doc.defaultView ?? window;
    let timer = 0;
    const clearLater = (delay: number) => {
      view.clearTimeout(timer);
      timer = view.setTimeout(() => {
        noSavePress.current = false;
      }, delay);
    };
    const press = (event: Event) => {
      const target = event.target;
      noSavePress.current =
        target instanceof Element &&
        target.closest('[data-editor-no-save]') !== null;
      // 桌面上失焦发生在这次按下的同一轮事件里；触屏要等到 click。兜底一秒后清掉。
      clearLater(1000);
    };
    const settle = () => clearLater(0);
    doc.addEventListener('pointerdown', press, true);
    doc.addEventListener('click', settle, true);
    doc.addEventListener('pointercancel', settle, true);
    return () => {
      doc.removeEventListener('pointerdown', press, true);
      doc.removeEventListener('click', settle, true);
      doc.removeEventListener('pointercancel', settle, true);
      view.clearTimeout(timer);
    };
  }, []);
  useEffect(() => {
    if (!mount.current) return;
    let view: EditorView | null = null;
    let active = true;
    const fail = () =>
      queueMicrotask(() => {
        if (active) setFailure('编辑器暂不可用，已保留草稿并改为只读文本。');
      });
    const nonce =
      mount.current.ownerDocument.head.querySelector<HTMLMetaElement>(
        'meta[name="studiolo-style-nonce"]',
      )?.content ?? '';
    try {
      view = new EditorView({
        parent: mount.current,
        state: EditorState.create({
          doc: current.current.value,
          extensions: [
            // 只接受服务端每次 HTML 响应注入的 nonce，开发占位符不授予样式权限。
            ...(nonce.length === 64 && /^[0-9a-f]{64}$/.test(nonce)
              ? [EditorView.cspNonce.of(nonce)]
              : []),
            EditorState.lineSeparator.of('\n'),
            history(),
            markdown(),
            EditorView.lineWrapping,
            keymap.of([...defaultKeymap, ...historyKeymap]),
            editable.of([
              EditorState.readOnly.of(current.current.readOnly),
              EditorView.editable.of(!current.current.readOnly),
            ]),
            EditorView.contentAttributes.of({
              'aria-label': 'Markdown 源码',
              spellcheck: 'false',
            }),
            EditorState.transactionFilter.of((transaction) => {
              if (
                transaction.docChanged &&
                transaction.annotation(Transaction.addToHistory) !== false &&
                !current.current.canChange(transaction.newDoc.toString())
              ) {
                queueMicrotask(() => current.current.onLimit());
                return [];
              }
              return transaction;
            }),
            EditorView.updateListener.of((update) => {
              if (
                update.docChanged &&
                !update.transactions.every(
                  (transaction) =>
                    transaction.annotation(Transaction.addToHistory) === false,
                )
              ) {
                current.current.onComposition(update.view.composing);
                current.current.onChange(update.state.sliceDoc());
              }
            }),
            EditorView.domEventHandlers({
              blur: (event) => {
                const next = event.relatedTarget;
                const intoNoSave =
                  next instanceof Element
                    ? next.closest('[data-editor-no-save]') !== null
                    : noSavePress.current;
                if (!intoNoSave) current.current.onBlur();
              },
              compositionstart: () => {
                current.current.onComposition(true);
              },
              compositionend: () => {
                queueMicrotask(() => current.current.onComposition(false));
              },
            }),
            EditorView.exceptionSink.of(fail),
          ],
        }),
      });
      instance.current = view;
    } catch {
      fail();
    }
    return () => {
      active = false;
      instance.current = null;
      view?.destroy();
    };
  }, [editable]);
  useEffect(() => {
    const view = instance.current;
    if (!view) return;
    view.dispatch({
      effects: editable.reconfigure([
        EditorState.readOnly.of(props.readOnly),
        EditorView.editable.of(!props.readOnly),
      ]),
    });
    if (view.state.sliceDoc() !== props.value && !view.composing)
      view.dispatch({
        changes: { from: 0, to: view.state.doc.length, insert: props.value },
        annotations: Transaction.addToHistory.of(false),
      });
  }, [props.value, props.readOnly, editable]);
  // 外观全在 editor.css 里按 CodeMirror 的类名换令牌；这里不传主题扩展，编辑行为不受影响。
  return failure ? (
    <div className="editor-fallback">
      <p role="alert" className="editor-notice-error">
        {failure}
      </p>
      <pre>{props.value}</pre>
    </div>
  ) : (
    <div className="markdown-editor" ref={mount} />
  );
}
