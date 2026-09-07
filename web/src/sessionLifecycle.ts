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
  const visibility = () => {
    if (document.visibilityState === 'hidden') client.suspend();
    else client.resume();
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
  window.addEventListener('focus', client.resume);
  window.addEventListener('pageshow', pageshow);
  window.addEventListener('pagehide', client.suspend);
  document.addEventListener('visibilitychange', visibility);
  return () => {
    channel?.close();
    client.onChange(null);
    window.removeEventListener('storage', storage);
    window.removeEventListener('focus', client.resume);
    window.removeEventListener('pageshow', pageshow);
    window.removeEventListener('pagehide', client.suspend);
    document.removeEventListener('visibilitychange', visibility);
  };
}
