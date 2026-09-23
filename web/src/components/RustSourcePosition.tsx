import type { RustSourceLocation } from '../rustCallSites';
import '../styles/rust-call-sites.css';

/**
 * 仓库源码位置：路径、一基行号与 UTF-8 字节列（明写「字节列」，不冒充显示字符列）。
 * 只是仓库文本，不走 Vault 原文弹窗，所以不做成按钮。
 * 路径一段单独包一层：同一文件的表格与侧栏里只给眼睛省掉重复的路径，读屏照读整句。
 */
export function RustSourcePosition({ source }: { source: RustSourceLocation }) {
  return (
    <p className="rust-source-position">
      <span className="rust-source-position__file">
        仓库源码：<code>{source.path}</code> ·{' '}
      </span>
      第 {source.line} 行 · UTF-8 字节列 {source.column}
    </p>
  );
}
