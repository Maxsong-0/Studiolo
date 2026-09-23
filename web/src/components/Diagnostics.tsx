/**
 * 数据提示：工作台快照里缺失或无效的信息。T-130 起不再挂在每页底部，只在顶栏状态浮层里逐条列出；
 * 浮层本身就是展开动作，这里不再套一层折叠。
 */
export function Diagnostics({ messages }: { messages: string[] }) {
  return messages.length ? (
    <div className="data-notes">
      <ul className="diagnostics">
        {messages.map((message) => (
          <li key={message}>{message}</li>
        ))}
      </ul>
      <p>缺失信息保持未知，点击来源核对。</p>
    </div>
  ) : (
    <p className="data-notes-empty">没有数据提示。</p>
  );
}
