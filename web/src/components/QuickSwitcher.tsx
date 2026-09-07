import { useEffect, useRef, useState } from 'react';
import type { OpenSource, Project } from '../types';
import { useSearch } from '../useSearch';
import { useDialogSurface } from '../useDialogSurface';
import { Icon } from './Icon';
import { QueryResults } from './QueryResults';
import { SearchControls } from './SearchControls';

export function QuickSwitcher({
  projects,
  generation,
  onOpen,
  onClose,
  onRefresh,
}: {
  projects: Project[];
  generation: number;
  onOpen: OpenSource;
  onClose: () => void;
  onRefresh: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const { dismiss } = useDialogSurface(dialog, onClose);
  const [query, setQuery] = useState('');
  const [project, setProject] = useState('');
  const search = useSearch(query, 'path', project, generation);
  useEffect(() => {
    dialog.current?.querySelector('input')?.focus();
  }, []);
  return (
    <dialog
      ref={dialog}
      className="quick-switcher"
      data-quick-switcher
      aria-labelledby="quick-title"
      onCancel={(event) => {
        event.preventDefault();
        dismiss();
      }}
      onKeyDown={(event) => {
        if (event.nativeEvent.isComposing) return;
        const buttons = [
          ...(dialog.current?.querySelectorAll<HTMLButtonElement>(
            '[data-search-result]',
          ) ?? []),
        ];
        if (!buttons.length) return;
        const index = buttons.indexOf(
          document.activeElement as HTMLButtonElement,
        );
        if (
          (event.key === 'ArrowDown' || event.key === 'ArrowUp') &&
          !(event.target instanceof HTMLSelectElement)
        ) {
          event.preventDefault();
          buttons[
            event.key === 'ArrowDown'
              ? (index + 1) % buttons.length
              : index <= 0
                ? buttons.length - 1
                : index - 1
          ].focus();
        } else if (
          event.key === 'Enter' &&
          event.target instanceof HTMLInputElement
        ) {
          event.preventDefault();
          buttons[0].click();
        }
      }}
    >
      <header className="quick-header">
        <h2 id="quick-title">快速切换</h2>
        <button
          className="icon-button"
          onClick={dismiss}
          aria-label="关闭快速切换"
          data-dialog-dismiss
        >
          <Icon name="close" />
        </button>
      </header>
      <p className="quick-hint">
        按完整路径查找；空格分隔的片段需全部匹配。↑ ↓ 选择，Enter 打开，Esc
        关闭。
      </p>
      <SearchControls
        query={query}
        onQuery={setQuery}
        project={project}
        onProject={setProject}
        projects={projects}
        quick
      />
      <div className="quick-results">
        <QueryResults
          search={search}
          onOpen={onOpen}
          onRefresh={onRefresh}
          emptyMessage="没有匹配的文档，请调整路径片段或项目范围。"
        />
      </div>
    </dialog>
  );
}
