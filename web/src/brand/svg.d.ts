/**
 * 只给带 `?no-inline` 的 svg 导入声明类型，值是构建后的文件地址。
 *
 * Vite 默认把 4KB 以下的资源内嵌成 `data:` 地址，而本站的 CSP 是 `img-src 'self'`，`data:` 图片一律被拦。
 * 不带这个后缀的 svg 导入故意不给类型：写了就过不了类型检查，免得小图标悄悄变成内嵌地址。
 */
declare module '*.svg?no-inline' {
  const url: string;
  export default url;
}
