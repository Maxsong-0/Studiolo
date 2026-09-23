/**
 * 旧名：快速切换已升级为 ⌘K 动作面板（T-130，设计规范第 8 节）。
 * 实现与 BUG-011 的修法在 commands/useCommandPalette.ts；这里保留旧文件名的再导出，免得别处的旧链接断掉。
 */
export { useCommandPalette as useQuickSwitcher } from './commands/useCommandPalette';
