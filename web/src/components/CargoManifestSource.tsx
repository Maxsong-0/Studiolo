import type { SourceLocation } from '../types';

export function CargoManifestSource({
  source,
  label = '仓库清单来源',
}: {
  source: SourceLocation;
  label?: string;
}) {
  return (
    <p className="cargo-manifest-source">
      {label}：<code>{source.path}</code> · 第 {source.line} 行
    </p>
  );
}
