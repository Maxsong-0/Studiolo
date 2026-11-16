import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';
import { studioloFixtures } from './dev/fixturePlugin.ts';
import { themeBoot } from './dev/themeBootPlugin.ts';

export default defineConfig({
  // 假数据插件只在 vite serve 且 STUDIOLO_FIXTURES=1 时生效，见 dev/fixturePlugin.ts。
  // 深浅色的启动脚本构建和开发都要：首屏之前把手动选的深浅色套上，见 dev/themeBootPlugin.ts。
  plugins: [react(), studioloFixtures(), themeBoot()],
  server: {
    host: '127.0.0.1',
    proxy: { '/api': { target: 'http://127.0.0.1:4317', changeOrigin: true } },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test/setup.ts'],
    restoreMocks: true,
    clearMocks: true,
  },
});
