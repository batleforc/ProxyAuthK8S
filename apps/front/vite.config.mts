/// <reference types='vitest' />
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
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
  plugins: [vue()],
  test: {
    name: 'front',
    environment: 'jsdom',
    include: ['src/**/*.{spec,test}.ts'],
    reporters: ['default'],
    // maz-ui ships components that import their own .css files: let Vite
    // transform them instead of Node, so component tests can mount them.
    server: { deps: { inline: [/maz-ui/] } },
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
