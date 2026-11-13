import { bindings, defineConfig, triggers } from 'cf/config';

export default defineConfig({
  worker: {
    name: 'studiolo-alert-heartbeats',
    entrypoint: './src/worker.mjs',
    compatibilityDate: '2026-11-13',
    compatibilityFlags: ['nodejs_compat'],
    previewUrls: false,
    triggers: [triggers.scheduled({ schedule: '* * * * *' })],
    env: {
      DB: bindings.d1({ name: 'studiolo-alert-heartbeats' }),
      NODE_108_TOKEN: bindings.secret(),
      NODE_169_TOKEN: bindings.secret(),
      PUSHOVER_TOKEN: bindings.secret(),
      PUSHOVER_USER: bindings.secret(),
      BARK_DEVICE_KEY: bindings.secret(),
    },
    observability: {
      enabled: true,
      redactQueryString: true,
      logs: { enabled: true, headSamplingRate: 1 },
      traces: { enabled: true, headSamplingRate: 1 },
    },
  },
});
