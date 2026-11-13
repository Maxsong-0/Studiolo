/** 人工附件，只有这些公开夹具字节会被开发入口返回。 */
const png = Uint8Array.from(
  atob(
    'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aX1sAAAAASUVORK5CYII=',
  ),
  (c) => c.charCodeAt(0),
);
const text = new TextEncoder();
export const ATTACHMENT_BYTES = new Map([
  [
    '项目/Studiolo/01-框架/视觉方向第三轮.png',
    { bytes: png, type: 'image/png' },
  ],
  [
    '项目/Brief 简报站/05-版本/v0.3.0 发布截图.png',
    { bytes: png, type: 'image/png' },
  ],
  [
    '项目/Studiolo/01-框架/附件文本.csv',
    {
      bytes: text.encode('标题,说明\n预览,<script>只是文本</script>\n'),
      type: 'text/plain; charset=utf-8',
    },
  ],
  [
    '项目/Studiolo/01-框架/下载演示.html',
    {
      bytes: text.encode('<html><body>仅下载的人工夹具</body></html>'),
      type: 'application/octet-stream',
    },
  ],
]);
