import react from '@vitejs/plugin-react';
import { defineConfig } from 'vitest/config';
import { studioloFixtures } from './dev/fixturePlugin.ts';

export default defineConfig({
  // 假数据插件只在 vite serve 且 STUDIOLO_FIXTURES=1 时生效，见 dev/fixturePlugin.ts。
  plugins: [react(), studioloFixtures()],
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
