import { useState } from 'react';
import type { ProjectOperationsData } from '../projectOperations';
import type { RustCallGraph } from '../rustCallSites';
import type { OpenSource } from '../types';
import { RustCallFileView } from './RustCallFileView';
import { RustFileDeclarations } from './RustFileDeclarations';

export function RustCallSitesView({
  graph,
  data,
  onOpen,
}: {
  graph: RustCallGraph;
  data: ProjectOperationsData;
  onOpen: OpenSource;
}) {
  const [selection, setSelection] = useState('');
  const selected =
    graph.files.find((file) => file.path === selection) ?? graph.files[0];
  return (
    <div className="rust-call-layout">
      <div className="rust-files" role="group" aria-label="Rust 白名单文件">
        {graph.files.map((file) => (
          <button
            key={file.path}
            aria-pressed={selected.path === file.path}
            aria-label={`选择 Rust 文件 ${file.path}`}
            onClick={() => setSelection(file.path)}
          >
            <code>{file.path}</code>
            <span>{file.bytes} 字节</span>
          </button>
        ))}
      </div>
      <div className="rust-file-content">
        <h3 className="rust-selected-file">
          <code>{selected.path}</code>
        </h3>
        <RustCallFileView
          key={`${selected.path}:${data.rust_call_sites?.links?.batch_id ?? 'legacy'}`}
          graph={graph}
          file={selected}
          links={data.rust_call_sites?.links ?? undefined}
        />
        <RustFileDeclarations
          path={selected.path}
          data={data}
          onOpen={onOpen}
        />
      </div>
    </div>
  );
}
