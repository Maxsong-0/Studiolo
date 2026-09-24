import { useEffect, useId, useLayoutEffect, useRef, useState } from 'react';
import type { KeyboardEvent, ReactNode } from 'react';
import { arrangeCommands, documentEntries } from '../commands/arrange';
import type { MatchRange } from '../commands/match';
import {
  buildCommands,
  COMMAND_GROUPS,
  executeCommand,
} from '../commands/registry';
import type {
  Command,
  CommandContext,
  CommandGroup,
} from '../commands/registry';
import {
  DOCUMENT_HIT_LIMIT,
  DOCUMENT_QUERY_MIN,
  useDocumentHits,
} from '../commands/useDocumentHits';
import type { DocumentHits } from '../commands/useDocumentHits';
import { usePaletteOverview } from '../commands/usePaletteOverview';
import type { PaletteOverview } from '../commands/usePaletteOverview';
import { KeyHint, StatusMark, Surface } from '../ui';
import { useDialogSurface } from '../useDialogSurface';
import '../styles/palette.css';

/** 面板从外面拿的上下文；黑板汇总由面板打开时自己读。 */
export type CommandPaletteContext = Omit<CommandContext, 'overview'>;

const GLYPHS: Record<CommandGroup, string> = {
  action: '→',
  navigate: '›',
  document: '¶',
};

const GROUP_TITLES = Object.fromEntries(
  COMMAND_GROUPS.map((group) => [group.id, group.title]),
) as Record<CommandGroup, string>;

/** 名字里命中的片段用红色下划线标出（设计规范第 8 节）。 */
function Highlighted({
  text,
  ranges,
}: {
  text: string;
  ranges: readonly MatchRange[];
}) {
  if (!ranges.length) return text;
  const parts: ReactNode[] = [];
  let cursor = 0;
  for (const [start, end] of ranges) {
    if (start > cursor) parts.push(text.slice(cursor, start));
    parts.push(
      <mark key={start} className="palette-match">
        {text.slice(start, end)}
      </mark>,
    );
    cursor = end;
  }
  if (cursor < text.length) parts.push(text.slice(cursor));
  return parts;
}

/**
 * ⌘K 动作面板（设计规范第 8 节，画布 R3_palette）：动作 · 跳转 · 文档三组。
 * 焦点始终留在输入框（combobox），↑↓ Home End 在结果里移动，↵ 执行，Esc 关闭并把焦点还给打开前的位置。
 * 不可逆动作只显示红色虚线框的「执行预览」，回车只带人去拥有该动作的界面，面板自己绝不调接口。
 * ⌘↵「在新格打开」不实现：Studiolo 没有分栏或侧栏，文档与编辑器都是单一的全屏覆盖层。
 */
export function CommandPalette({
  context,
  generation,
  onClose,
  onRefresh,
}: {
  context: CommandPaletteContext;
  /** 当前快照代次：文档搜索只认这一代的结果。 */
  generation: number;
  onClose: () => void;
  /** 文档搜索发现快照已更新（409）时的「刷新工作台」。 */
  onRefresh: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const id = useId();
  const [query, setQuery] = useState('');
  const [scope, setScope] = useState('');
  const [activeId, setActiveId] = useState<string | null>(null);
  const board = usePaletteOverview();
  const documents = useDocumentHits(query, scope, generation);

  // Esc 与 ⌘K 再按一次都走 dialog 的 cancel：拦下浏览器的直接关闭，改走可中断的收起并归还焦点。
  // 用原生监听而不是 onCancel 属性：React 会让 cancel 沿组件树冒泡，内层对话框的 cancel 也会触发外层的 onCancel；
  // 对话框的原生 cancel 不冒泡，内层的到不了这里。在布局阶段紧跟 showModal 挂上，不留空档。
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

  useEffect(() => {
    input.current?.focus();
  }, []);

  const ctx: CommandContext = {
    ...context,
    overview: board.kind === 'ready' ? board.overview : null,
  };
  const sections = arrangeCommands(buildCommands(ctx), query, {
    tasksTruncated: board.kind === 'ready' && board.overview.itemsTruncated,
  });
  if (documents.phase === 'ready' && documents.hits.length)
    sections.push({
      group: 'document',
      entries: documentEntries(documents.hits, query),
    });
  const options = sections.flatMap((section) => section.entries);
  const found =
    activeId === null
      ? 0
      : options.findIndex((entry) => entry.command.id === activeId);
  const activeIndex = options.length ? Math.max(found, 0) : -1;
  const active = activeIndex >= 0 ? options[activeIndex].command : null;
  const listId = `${id}-list`;
  const previewId = `${id}-preview`;
  const optionId = (index: number) => `${id}-option-${index}`;

  function run(command: Command) {
    // 先关面板（焦点随之还给入口按钮），再执行：打开的原文、状态浮层会从那里接过焦点。
    onClose();
    executeCommand(command, ctx);
  }

  function select(index: number) {
    setActiveId(options[index].command.id);
    document
      .getElementById(optionId(index))
      ?.scrollIntoView({ block: 'nearest' });
  }

  function onKeyDown(event: KeyboardEvent<HTMLInputElement>) {
    if (event.nativeEvent.isComposing) return;
    const count = options.length;
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      if (!count) return;
      event.preventDefault();
      select(
        event.key === 'ArrowDown'
          ? (activeIndex + 1) % count
          : (activeIndex - 1 + count) % count,
      );
    } else if (event.key === 'Home' || event.key === 'End') {
      if (!count) return;
      event.preventDefault();
      select(event.key === 'Home' ? 0 : count - 1);
    } else if (event.key === 'Enter') {
      // ⌘↵／Ctrl+↵ 没有「新格」可开，不当作普通回车执行。
      if (event.metaKey || event.ctrlKey) return;
      event.preventDefault();
      if (active) run(active);
    }
  }

  let index = 0;
  const groups = sections.map((section) => {
    const titleId = `${id}-group-${section.group}`;
    return (
      <div
        key={section.group}
        role="group"
        aria-labelledby={titleId}
        className="palette-group"
      >
        <div id={titleId} role="presentation" className="palette-group-title">
          {GROUP_TITLES[section.group]}
        </div>
        {section.entries.map((entry) => {
          const position = index++;
          const { command } = entry;
          const selected = position === activeIndex;
          const hintId = `${optionId(position)}-hint`;
          const described = [
            command.hint ? hintId : null,
            selected && command.irreversible ? previewId : null,
          ].filter(Boolean);
          return (
            <div
              key={command.id}
              id={optionId(position)}
              role="option"
              aria-selected={selected}
              // 名字只取命令名，说明与执行预览作描述：红线标记不会把名字拆成几段。
              aria-label={command.label}
              aria-describedby={described.join(' ') || undefined}
              className="palette-option"
              onPointerMove={() => {
                if (!selected) setActiveId(command.id);
              }}
              // 点选不抢走输入框的焦点：面板始终以输入框为焦点。
              onMouseDown={(event) => event.preventDefault()}
              onClick={() => run(command)}
            >
              <span className="palette-option-glyph" aria-hidden="true">
                {GLYPHS[command.group]}
              </span>
              <span className="palette-option-text">
                <span className="palette-option-label">
                  <Highlighted text={command.label} ranges={entry.ranges} />
                </span>
                {command.hint ? (
                  <span id={hintId} className="palette-option-hint">
                    {command.hint}
                  </span>
                ) : null}
              </span>
              <span className="palette-option-key" aria-hidden="true">
                {selected ? (
                  '↵'
                ) : command.irreversible ? (
                  <span className="palette-option-tag">预览</span>
                ) : null}
              </span>
            </div>
          );
        })}
      </div>
    );
  });

  const preview = active?.irreversible ?? null;

  return (
    <Surface
      as="dialog"
      ref={dialog}
      appear={false}
      className="palette"
      aria-labelledby={`${id}-title`}
      // 与 COMMAND_PALETTE_ATTRIBUTE 同名：快捷键据此认出「已开着的是面板本身」还是别的模态。
      data-command-palette=""
    >
      <h2 id={`${id}-title`} className="sr-only">
        动作面板
      </h2>
      <div className="palette-head">
        <span className="palette-head-key" aria-hidden="true">
          ⌘K
        </span>
        <input
          ref={input}
          className="palette-input"
          type="text"
          role="combobox"
          aria-label="跳转、搜索或执行动作"
          aria-autocomplete="list"
          aria-controls={listId}
          aria-expanded={options.length > 0}
          aria-activedescendant={
            activeIndex >= 0 ? optionId(activeIndex) : undefined
          }
          placeholder="跳转、搜索或执行动作"
          autoComplete="off"
          spellCheck={false}
          value={query}
          onChange={(event) => {
            setQuery(event.target.value);
            setActiveId(null);
          }}
          onKeyDown={onKeyDown}
        />
        <label className="palette-scope">
          <span aria-hidden="true">范围</span>
          <select
            aria-label="文档搜索范围"
            value={scope}
            onChange={(event) => setScope(event.target.value)}
          >
            <option value="">全部</option>
            {context.projects.map((project) => (
              <option key={project.id} value={project.id}>
                {project.name}
              </option>
            ))}
          </select>
        </label>
        <span className="palette-count" role="status">
          {options.length} 条结果
        </span>
      </div>
      <div className="palette-body">
        <div
          id={listId}
          role="listbox"
          aria-label="动作、跳转与文档"
          className="palette-list"
        >
          {groups}
        </div>
        <div className="palette-notes">
          {!options.length && query.trim() ? (
            <p role="status">没有匹配的动作或跳转。</p>
          ) : null}
          <BoardNote board={board} />
          <DocumentNote documents={documents} onRefresh={onRefresh} />
        </div>
      </div>
      {preview ? (
        <div className="palette-preview" id={previewId}>
          <p className="palette-preview-label">执行预览</p>
          <p>{preview.preview}</p>
          <p className="palette-preview-route">
            按 ↵ 只会前往「{preview.surfaceLabel}
            」，面板不发任何请求；确认仍在那里完成。
          </p>
        </div>
      ) : null}
      <footer className="palette-foot">
        <span className="palette-foot-keys">
          <KeyHint keys={['↑', '↓']} /> 选择
        </span>
        <span className="palette-foot-keys">
          <KeyHint keys={['↵']} /> 执行
        </span>
        <button
          type="button"
          className="palette-close"
          aria-label="关闭动作面板"
          data-dialog-dismiss
          onClick={dismiss}
        >
          <KeyHint keys={['esc']} /> 关闭
        </button>
      </footer>
    </Surface>
  );
}

/**
 * 黑板任务这几行的来历：读取中、未启用、读不到，各说一句；读到了就不占地方。
 * 读取中只给眼睛看，不进播报：每次打开面板都会闪一下，读屏逐次念出来只是噪音。
 */
function BoardNote({ board }: { board: PaletteOverview }) {
  if (board.kind === 'ready') return null;
  if (board.kind === 'loading')
    return (
      <p>
        <StatusMark state="pending">正在读取黑板任务…</StatusMark>
      </p>
    );
  if (board.kind === 'disabled')
    return (
      <p>
        <StatusMark state="dormant">
          本机未启用黑板，没有黑板任务可跳。
        </StatusMark>
      </p>
    );
  return (
    <p role="status">
      <StatusMark state="alert">黑板任务读取失败：{board.message}</StatusMark>
    </p>
  );
}

/** 文档组的状态：字数不够、搜索中、失败（带重试或刷新）、没有命中、只列了前几条。 */
function DocumentNote({
  documents,
  onRefresh,
}: {
  documents: DocumentHits & { retry: () => void };
  onRefresh: () => void;
}) {
  switch (documents.phase) {
    case 'idle':
      return (
        <p>输入 {DOCUMENT_QUERY_MIN} 个字以上，同时搜索文档正文与路径。</p>
      );
    case 'invalid':
      return <p role="alert">{documents.error}</p>;
    case 'loading':
      return <p role="status">正在搜索当前快照…</p>;
    case 'error':
      return (
        <div role="alert" className="palette-note-error">
          <p>
            <StatusMark state="alert">
              {documents.status === 409
                ? '快照已更新，请刷新工作台。'
                : documents.status === 429
                  ? '搜索服务正忙，请稍后重试。'
                  : ''}
              {documents.error}
            </StatusMark>
          </p>
          <button
            type="button"
            className="palette-note-action"
            onClick={documents.status === 409 ? onRefresh : documents.retry}
          >
            {documents.status === 409 ? '刷新工作台' : '重试搜索'}
          </button>
        </div>
      );
    case 'ready':
      if (!documents.hits.length)
        return <p role="status">没有匹配的文档，请换个词或调整范围。</p>;
      if (documents.total <= DOCUMENT_HIT_LIMIT && !documents.truncated)
        return null;
      return (
        <p>
          文档命中{documents.truncated ? '超过 ' : ' '}
          {documents.total} 条，这里列前 {DOCUMENT_HIT_LIMIT}{' '}
          条；完整结果请到「文档」视图搜索。
        </p>
      );
  }
}
