/**
 * 深浅色的启动脚本（T-199）：人手动选的深浅色记在本机存储里，要在首屏之前套到 `<html>` 上，
 * 否则选的和系统相反时，页面会先按系统那一套画一下再换过来（应用自己的脚本是模块，排在首屏之后才跑）。
 *
 * 做法：一段十来行的经典脚本，放进 `<head>` 最前面，解析到它就停下来执行，早于任何绘制。
 * - 不能内联：页面的 CSP 是 `script-src 'self'`，内联脚本不执行。所以它是 `/assets/` 下的一个文件。
 * - 文件名带内容哈希（`theme-boot-<8 位>.js`）：服务端只对这种名字发长期缓存（crates/local-server
 *   的 security/cache.rs），内容一变名字就变。哈希由脚本原文算出，不依赖打包器的钩子顺序。
 * - 构建时由 `generateBundle` 放进产物；开发服务器照同一个地址应答。
 * - 键、属性名和取值都从 `src/theme/preference.ts` 拿，和应用用的是同一份，不另写。
 *
 * 脚本只做一件事：本机存储里是 `light` 或 `dark` 就写到 `<html data-theme>`；读不了就什么都不做
 * （应用启动后 `startTheme` 会再套一次）。
 */
import { createHash } from 'node:crypto';
import type { Plugin } from 'vite';
import {
  MANUAL_THEMES,
  THEME_ATTRIBUTE,
  THEME_KEY,
} from '../src/theme/preference.ts';

/** 启动脚本的原文。只用老写法（var、indexOf）：它不经过打包器转换。 */
export function themeBootSource(): string {
  const key = JSON.stringify(THEME_KEY);
  const attribute = JSON.stringify(THEME_ATTRIBUTE);
  const allowed = JSON.stringify(MANUAL_THEMES);
  return (
    `(function(){try{var t=localStorage.getItem(${key});` +
    `if(${allowed}.indexOf(t)!==-1)` +
    `document.documentElement.setAttribute(${attribute},t)}catch(e){}})();\n`
  );
}

/** 产物里的相对路径：`assets/theme-boot-<内容哈希前 8 位，URL 安全的 base64>.js`。 */
export function themeBootPath(source: string = themeBootSource()): string {
  const hash = createHash('sha256')
    .update(source)
    .digest('base64url')
    .slice(0, 8);
  return `assets/theme-boot-${hash}.js`;
}

export function themeBoot(): Plugin {
  const source = themeBootSource();
  const path = themeBootPath(source);
  return {
    name: 'studiolo:theme-boot',
    transformIndexHtml() {
      return [
        { tag: 'script', attrs: { src: `/${path}` }, injectTo: 'head-prepend' },
      ];
    },
    generateBundle() {
      this.emitFile({ type: 'asset', fileName: path, source });
    },
    configureServer(server) {
      server.middlewares.use(`/${path}`, (_request, response) => {
        response.setHeader('Content-Type', 'text/javascript; charset=utf-8');
        response.setHeader('Cache-Control', 'no-store');
        response.end(source);
      });
    },
  };
}
