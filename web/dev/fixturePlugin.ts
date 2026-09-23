/**
 * 开发用假数据模式（T-130 A4）：
 *
 *     STUDIOLO_FIXTURES=1 npm --prefix web run dev
 *
 * 没有任何后端时，`/api/*` 由 `web/dev/fixtureApi.ts` 在 Vite 代理之前应答，全部视图（驾驶舱、
 * 项目运作、文档、协作黑板与跨项目汇总、时间回放、执行、策略、健康、账户）都用逼真的假数据渲染。
 *
 * 边界（设计规范第 12 节不变量 5）：
 * - `apply: 'serve'`：只在 `vite serve` 下挂；`vite build` / `vite preview` 不经过这里，
 *   应用源码也不引用 `web/dev/`，所以生产构建产物里没有任何假数据。
 * - 只有环境变量 `STUDIOLO_FIXTURES` 恰好等于 `1` 才启用；没设时 `configureServer` 立即返回，
 *   不注册中间件、不打印任何东西，开发服务器与原来完全一致（请求照旧走代理到 127.0.0.1:4317）。
 * - Vitest 也会起 Vite 服务器：它里面一律不启用，测试结果不受这个开关影响。
 */
import type { IncomingMessage, ServerResponse } from 'node:http';
import type { Plugin, ViteDevServer } from 'vite';
import { createFixtureApi } from './fixtureApi.ts';
import type { FixtureResponse } from './fixtureApi.ts';
import { FIXTURE_MARKER } from './fixtures/ids.ts';

export const FIXTURE_ENV = 'STUDIOLO_FIXTURES';
/** 保存正文最多 256 KiB，留足余量；超过即 413，不读完整个流。 */
const BODY_LIMIT = 2 * 1024 * 1024;
const BANNER =
  '假数据模式：STUDIOLO_FIXTURES=1，/api/* 全部由 web/dev 假数据应答，不连后端；写操作只改内存，重启即复原。';

export function fixturesEnabled(env: NodeJS.ProcessEnv = process.env) {
  return env[FIXTURE_ENV] === '1' && !env.VITEST;
}

export function studioloFixtures(): Plugin {
  return {
    name: 'studiolo:dev-fixtures',
    apply: 'serve',
    configureServer(server) {
      if (!fixturesEnabled()) return;
      installFixtures(server);
    },
  };
}

function installFixtures(server: ViteDevServer) {
  const api = createFixtureApi();
  // configureServer 里直接 use 的中间件排在 Vite 内置中间件（含 /api 代理）之前。
  server.middlewares.use((request, response, next) => {
    const url = request.url ?? '';
    if (
      url !== '/api' &&
      !url.startsWith('/api/') &&
      !url.startsWith('/api?')
    ) {
      next();
      return;
    }
    readBody(request).then(
      (raw) => {
        let body: unknown;
        if (raw.length) {
          try {
            body = JSON.parse(raw.toString('utf8'));
          } catch {
            send(response, {
              status: 400,
              body: { error: '请求正文不是有效 JSON。' },
            });
            return;
          }
        }
        send(
          response,
          api.handle({
            method: request.method ?? 'GET',
            url,
            headers: headers(request),
            body,
          }),
        );
      },
      () => send(response, { status: 413, body: { error: '请求正文过大。' } }),
    );
  });
  const color = process.stdout.isTTY && !process.env.NO_COLOR;
  const line = color ? `\x1b[1m\x1b[33m  ${BANNER}\x1b[0m` : `  ${BANNER}`;
  const printUrls = server.printUrls.bind(server);
  server.printUrls = () => {
    printUrls();
    server.config.logger.warn(line);
  };
}

/** Node 已把头名转成小写；重复出现的头按「无效」处理，与后端拒绝重复头一致。 */
function headers(request: IncomingMessage) {
  const result: Record<string, string | undefined> = {};
  for (const [name, value] of Object.entries(request.headers))
    result[name] = typeof value === 'string' ? value : undefined;
  return result;
}

function readBody(request: IncomingMessage): Promise<Buffer> {
  return new Promise((resolve, reject) => {
    const chunks: Buffer[] = [];
    let size = 0;
    request.on('data', (chunk: Buffer) => {
      size += chunk.length;
      if (size > BODY_LIMIT) {
        request.destroy();
        reject(new Error('body too large'));
        return;
      }
      chunks.push(chunk);
    });
    request.on('end', () => resolve(Buffer.concat(chunks)));
    request.on('error', reject);
  });
}

function send(response: ServerResponse, result: FixtureResponse) {
  response.statusCode = result.status;
  response.setHeader('Cache-Control', 'no-store');
  response.setHeader('X-Studiolo-Fixture', FIXTURE_MARKER);
  if (result.body === undefined) {
    response.end();
    return;
  }
  response.setHeader('Content-Type', 'application/json; charset=utf-8');
  response.end(JSON.stringify(result.body));
}
