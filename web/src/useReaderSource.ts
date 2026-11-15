import { useCallback, useState } from 'react';
import type { SourceLocation, View } from './types';
import { locatable } from './viewLocation';
import type { ViewLocation, ViewSlot } from './viewLocation';

interface Opened {
  source: SourceLocation;
  /** 在哪个视图里打开的：换了视图就不再算数，不会在别处突然弹出来。 */
  view: View;
  /** 路径是否已经记进视图位置的 `item` 格。 */
  linked: boolean;
}

/**
 * 阅读器在看哪一篇（T-209，Tillerwright T-046 登记的「文档视图深链到单个文件」）。
 *
 * 文档视图里，路径同时记在视图位置的 `item` 格，并且以位置为准：刷新、会话重新核对后的整页重挂、
 * 把链接发到另一台设备、手改地址，都落在同一篇上；位置里的路径没了，阅读器也跟着关。
 * 别的视图的 `item` 另有含义，在那里打开阅读器不写位置，照旧只活在内存里。
 *
 * 位置里只有文件路径：它是外部输入，只当文本用，读不读得到仍由读取接口照常鉴权。
 * 行号、正文和搜索词都不进位置。放不进一格的路径（超长等）不记位置，阅读器照常打开。
 */
export function useReaderSource(
  location: ViewLocation,
  select: (patch: Partial<Record<ViewSlot, string | null>>) => void,
) {
  const [opened, setOpened] = useState<Opened | null>(null);
  const inDocuments = location.view === 'documents';
  const mine = opened?.view === location.view ? opened : null;
  let source: SourceLocation | null;
  if (!inDocuments || (mine && !mine.linked)) source = mine?.source ?? null;
  else if (location.item === null) source = null;
  else
    source =
      mine?.source.path === location.item
        ? mine.source
        : { path: location.item, line: 1 };

  const view = location.view;
  const open = useCallback(
    (next: SourceLocation) => {
      const linked = view === 'documents' && locatable(next.path);
      setOpened({ source: next, view, linked });
      if (view === 'documents') select({ item: linked ? next.path : null });
    },
    [view, select],
  );
  const close = useCallback(() => {
    setOpened(null);
    if (view === 'documents') select({ item: null });
  }, [view, select]);
  return { source, open, close };
}
