import { useEffect, useId, useLayoutEffect, useRef } from 'react';
import type { ReactNode } from 'react';
import { Icon } from '../components/Icon';
import './shell.css';

/** 一次带路：展开之后把焦点交给面板里匹配 selector 的入口；serial 每次加一，同一个入口也能再带一次。 */
export interface ToolsReveal {
  selector: string;
  serial: number;
}

/**
 * 顶栏「更多」菜单（T-199；设计规范第 5 节）：账户、最近删除、内存草稿和深浅色切换都收在这里，
 * 顶栏在任何宽度都只占一行。它是「展开／收起」，不是模态：Esc 收起并把焦点还给「更多」，
 * 点外面、键盘把焦点移出去也收起。
 *
 * 入口和它们各自的对话框是同一个组件画出来的，所以面板里的东西**一直挂着**，收起只靠样式：
 * 面板收着时自己不占盒子，只把直接子按钮藏掉；对话框在浏览器顶层照常显示（shell.css）。
 * 这样 ⌘K 的「打开最近删除」在菜单收着时也打得开，入口组件自己的状态也不会因为收起而丢。
 *
 * 点了面板里的入口（面板的直接子按钮）：菜单收起，焦点先交给「更多」。随后打开的对话框把
 * 「更多」记成打开它的东西，关掉时焦点回到这里，不会落到一个已经藏起来的按钮上。
 * 深浅色的三格在自己的分组里，点了不收菜单——选完还能接着看、接着换。
 */
export function ToolsMenu({
  open,
  onOpenChange,
  reveal,
  note,
  children,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  /** 最近一次带路；null 表示还没带过。 */
  reveal: ToolsReveal | null;
  /** 菜单里有事要人看（有内存草稿、文件操作待核对）：写在「更多」上；没有就不传。 */
  note?: string | null;
  children: ReactNode;
}) {
  const root = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLDivElement>(null);
  const panelId = useId();

  useEffect(() => {
    if (!open) return;
    function outside(event: PointerEvent) {
      if (!root.current?.contains(event.target as Node)) onOpenChange(false);
    }
    document.addEventListener('pointerdown', outside);
    return () => document.removeEventListener('pointerdown', outside);
  }, [open, onOpenChange]);

  // 带路：这一轮里菜单已经展开，入口接得住焦点了。
  useLayoutEffect(() => {
    if (reveal === null) return;
    panel.current?.querySelector<HTMLElement>(reveal.selector)?.focus();
  }, [reveal]);

  return (
    <div
      ref={root}
      className="shell-more"
      onKeyDown={(event) => {
        if (event.key !== 'Escape' || !open || event.nativeEvent.isComposing)
          return;
        // 对话框里的 Esc 是对话框自己的事（它们在 DOM 里也是面板的后代）。
        if (event.target instanceof Element && event.target.closest('dialog'))
          return;
        event.preventDefault();
        onOpenChange(false);
        trigger.current?.focus();
      }}
      onBlur={(event) => {
        // 键盘把焦点移出「更多」和面板才收起；点到面板里的文字（relatedTarget 为空）不算移出。
        const next = event.relatedTarget;
        if (open && next instanceof Node && !root.current?.contains(next))
          onOpenChange(false);
      }}
    >
      <button
        ref={trigger}
        type="button"
        className="shell-more-toggle"
        aria-expanded={open}
        aria-controls={panelId}
        aria-label={note ? `更多（${note}）` : '更多'}
        onClick={() => onOpenChange(!open)}
      >
        <Icon name="menu" />
        <span className="shell-more-label">更多</span>
        {note ? (
          <>
            <span className="shell-more-mark" aria-hidden="true">
              ■
            </span>
            <span className="shell-more-note">{note}</span>
          </>
        ) : null}
      </button>
      <div
        ref={panel}
        id={panelId}
        className="shell-tools"
        role="group"
        aria-label="更多"
        data-open={open ? 'true' : undefined}
        onClick={(event) => {
          if (!open || !(event.target instanceof Element)) return;
          const entry = event.target.closest('button');
          if (entry?.parentElement !== event.currentTarget) return;
          onOpenChange(false);
          trigger.current?.focus();
        }}
      >
        {children}
      </div>
    </div>
  );
}
