import type { SourceLocation } from '../types';
import '../styles/cargo-dependencies.css';

/**
 * 仓库清单来源：仓库相对路径＋一基行号。它是仓库文本，不是 Vault 原文入口，所以只显示、不做成按钮。
 * 标签单独包一层：包表格在宽屏由表头代替它（读屏仍会读到），窄屏逐格显示。
 */
export function CargoManifestSource({
  source,
  label = '仓库清单来源',
}: {
  source: SourceLocation;
  label?: string;
}) {
  return (
    <p className="cargo-manifest-source">
      <span className="cargo-manifest-source__label">{label}：</span>
      <code>{source.path}</code> · 第 {source.line} 行
    </p>
  );
}
