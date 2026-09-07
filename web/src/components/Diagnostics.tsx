export function Diagnostics({ messages }: { messages: string[] }) {
  return messages.length ? (
    <div className="data-notes">
      <details className="diagnostics">
        <summary>数据提示（{messages.length}）</summary>
        <ul>
          {messages.map((message) => (
            <li key={message}>{message}</li>
          ))}
        </ul>
      </details>
      <p>缺失信息保持未知，点击来源核对。</p>
    </div>
  ) : null;
}
