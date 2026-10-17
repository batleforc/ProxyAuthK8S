/// <reference types='vitest' />
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import tailwindcss from '@tailwindcss/vite';
import fs from 'node:fs';

let tls = undefined;

if (process.env.VITE_TLS_ENABLE === 'true') {
  const currentPath = import.meta.dirname;
  tls = {
    key: fs.readFileSync(`${currentPath}/../../.tls/key.pem`),
    cert: fs.readFileSync(`${currentPath}/../../.tls/cert.pem`),
  };
}

export default defineConfig(() => ({
  root: import.meta.dirname,
  // The repository's .env files (dev values, .env.production placeholders that
  // env.sh fills at startup), which nx used to load for every task.
  envDir: '../../',
  cacheDir: '../../node_modules/.vite/apps/front',
  server: {
    port: 4200,
    host: 'localhost',
    https: tls,
    allowedHosts: [
      '.k8s.local',
      '.local',
      'localhost',
      '.che-cluster.4.weebo.fr',
    ],
  },
  preview: {
    port: 4300,
    host: 'localhost',
  },
  plugins: [vue(), tailwindcss()],
  resolve: {
    // The ported batlehub primitives import through `@/`.
    alias: { '@': `${import.meta.dirname}/src` },
  },
  define: {
    // vue-i18n compiles messages just in time (no eval, so the CSP holds)
    // with the Composition API only.
    __VUE_I18N_FULL_INSTALL__: false,
    __VUE_I18N_LEGACY_API__: false,
    __INTLIFY_PROD_DEVTOOLS__: false,
  },
  test: {
    name: 'front',
    environment: 'jsdom',
    include: ['src/**/*.{spec,test}.ts'],
    reporters: ['default'],
  },
  build: {
    outDir: '../../dist/apps/front',
    emptyOutDir: true,
    reportCompressedSize: true,
    commonjsOptions: {
      transformMixedEsModules: true,
    },
  },
}));
