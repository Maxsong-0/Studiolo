import {
  Fragment,
  useId,
  useLayoutEffect,
  useRef,
  useSyncExternalStore,
} from 'react';
import { Icon } from '../components/Icon';
import { KeyHint, Surface } from '../ui';
import { useDialogSurface } from '../useDialogSurface';
import type { SingleKeysPreference } from './preference';
import { SHORTCUT_GROUPS, type KeyWay } from './shortcuts';
import '../styles/keys.css';

/** 一种按法：组合键是一个键帽组；一个接一个按的，中间写「再按」。 */
function Way({ way }: { way: KeyWay }) {
  if (!way.sequence) return <KeyHint keys={way.keys} />;
  return (
    <>
      {way.keys.map((key, index) => (
        <Fragment key={index}>
          {index > 0 ? <span className="keys-sheet__then">再按</span> : null}
          <KeyHint keys={[key]} />
        </Fragment>
      ))}
    </>
  );
}

/**
 * 快捷键一览（T-197；设计规范第 16 节）：按「?」或从 ⌘K 的「查看键盘快捷键」打开，再按一次「?」或 Esc 关闭。
 * 内容照 shortcuts.ts 的那张表画，和监听用的是同一张；底下是单键快捷键的开关，只记在这台浏览器里。
 */
export function KeyboardSheet({
  onClose,
  preference,
}: {
  onClose: () => void;
  /** 单键快捷键的开关：和全站监听看的是同一个（useGlobalKeys 给出的那一个）。 */
  preference: SingleKeysPreference;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const enabled = useSyncExternalStore(
    preference.subscribe,
    preference.enabled,
  );
  const id = useId();
  const titleId = `${id}-title`;
  const noteId = `${id}-note`;

  // Esc 和再按一次「?」都走 dialog 的 cancel：拦下浏览器的直接关闭，改走可中断的收起并归还焦点。
  // 用原生监听：对话框的原生 cancel 不冒泡，React 的 onCancel 会沿组件树冒泡（见 AttachmentInfo）。
  useLayoutEffect(() => {
    const element = dialog.current;
    if (!element) return;
    function cancel(event: Event) {
      event.preventDefault();
      dismiss();
    }
    element.addEventListener('cancel', cancel);
    return () => element.removeEventListener('cancel', cancel);
  }, [dismiss]);

  return (
    <Surface
      as="dialog"
      ref={dialog}
      appear={false}
      className="keys-sheet"
      aria-labelledby={titleId}
      // 与 KEYBOARD_SHEET_ATTRIBUTE 同名：全站监听据此认出「开着的弹层就是一览」，再按一次「?」把它关掉。
      data-keyboard-sheet=""
    >
      <header className="keys-sheet__header">
        <h2 id={titleId}>键盘快捷键</h2>
        <button
          type="button"
          className="icon-button"
          onClick={dismiss}
          aria-label="关闭键盘快捷键"
          data-dialog-dismiss
        >
          <Icon name="close" />
        </button>
      </header>
      <div className="keys-sheet__groups">
        {SHORTCUT_GROUPS.map((group, groupIndex) => {
          const off = group.single && !enabled;
          const headingId = `${id}-group-${groupIndex}`;
          return (
            <section
              key={group.title}
              className="keys-sheet__group"
              data-off={off ? 'true' : undefined}
              aria-labelledby={headingId}
            >
              <h3 id={headingId}>
                {group.title}
                <span className="keys-sheet__where">
                  {group.where}
                  {off ? ' · 已关闭' : ''}
                </span>
              </h3>
              <dl className="keys-sheet__rows">
                {group.rows.map((row) => (
                  <div key={row.does} className="keys-sheet__row">
                    <dt>
                      {row.ways.map((way, index) => (
                        <Fragment key={index}>
                          {index > 0 ? (
                            <span className="keys-sheet__or">或</span>
                          ) : null}
                          <Way way={way} />
                        </Fragment>
                      ))}
                    </dt>
                    <dd>{row.does}</dd>
                  </div>
                ))}
              </dl>
            </section>
          );
        })}
      </div>
      <div className="keys-sheet__toggle">
        <label>
          <input
            type="checkbox"
            checked={enabled}
            aria-describedby={noteId}
            onChange={(event) => preference.set(event.target.checked)}
          />
          <span>启用单键快捷键</span>
        </label>
        <p id={noteId} className="keys-sheet__note">
          关掉之后 J、K、H、G 和「?」都不再响应，⌘K
          不受影响，还能从它那里打开这张一览再开回来。这个开关只记在这台浏览器里。
          焦点在输入框里、或有弹层开着时，单键快捷键本来就不响应。
        </p>
      </div>
    </Surface>
  );
}
