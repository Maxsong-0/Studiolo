import { StatusMark } from '../ui';
import '../styles/freshness.css';

/**
 * 数据提示：工作台快照里缺失或无效的信息。T-130 起不再挂在每页底部，只在顶栏状态浮层里逐条列出；
 * 浮层本身就是展开动作，这里不再套一层折叠。等宽列表，一条一行，每条原文完整保留。
 * 样式与新鲜度同在 freshness.css（两块都只出现在状态浮层里）。
 */
export function Diagnostics({ messages }: { messages: string[] }) {
  return messages.length ? (
    <div className="status-notes">
      <ul className="status-notes-list">
        {messages.map((message) => (
          <li key={message}>{message}</li>
        ))}
      </ul>
      <p className="status-notes-hint">缺失信息保持未知，点击来源核对。</p>
    </div>
  ) : (
    <p className="status-notes-empty">
      <StatusMark state="ok">没有数据提示。</StatusMark>
    </p>
  );
}
