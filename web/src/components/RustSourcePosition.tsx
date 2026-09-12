import type { RustSourceLocation } from '../rustCallSites';

export function RustSourcePosition({ source }: { source: RustSourceLocation }) {
  return (
    <p className="rust-source-position">
      仓库源码：<code>{source.path}</code> · 第 {source.line} 行 · UTF-8 字节列{' '}
      {source.column}
    </p>
  );
}
