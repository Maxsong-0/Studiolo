import type { SessionClient } from './sessionClient';

const channelName = 'studiolo-session-change';

export function watchSession(client: SessionClient) {
  const channel =
    typeof window.BroadcastChannel === 'function'
      ? new window.BroadcastChannel(channelName)
      : null;
  const storage = (event: StorageEvent) => {
    if (event.key === channelName) client.changed();
  };
  // 切标签、切应用只触发「够一天了再核对」；隐藏时不再清内容（用户 2026-09-22 拍板，ADR-022）。
  // 页面真正离开（pagehide）与从往返缓存恢复（pageshow persisted）仍按原契约暂停与重新核对。
  const visibility = () => {
    if (document.visibilityState === 'visible') client.revisit();
  };
  const pageshow = (event: PageTransitionEvent) => {
    if (event.persisted) client.resume();
  };
  client.onChange(() => {
    if (channel) channel.postMessage('changed');
    else {
      try {
        localStorage.setItem(channelName, `${Date.now()}:${Math.random()}`);
      } catch {
        /* Focus and page restoration still revalidate without storage. */
      }
    }
  });
  if (channel) channel.onmessage = () => client.changed();
  else window.addEventListener('storage', storage);
  window.addEventListener('focus', client.revisit);
  window.addEventListener('pageshow', pageshow);
  window.addEventListener('pagehide', client.suspend);
  document.addEventListener('visibilitychange', visibility);
  return () => {
    channel?.close();
    client.onChange(null);
    window.removeEventListener('storage', storage);
    window.removeEventListener('focus', client.revisit);
    window.removeEventListener('pageshow', pageshow);
    window.removeEventListener('pagehide', client.suspend);
    document.removeEventListener('visibilitychange', visibility);
  };
}
