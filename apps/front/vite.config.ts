/// <reference types='vitest' />
import { defineConfig } from 'vite';
import vue from '@vitejs/plugin-vue';
import { nxViteTsPaths } from '@nx/vite/plugins/nx-tsconfig-paths.plugin';
import { nxCopyAssetsPlugin } from '@nx/vite/plugins/nx-copy-assets.plugin';
import fs from 'fs';

let tls = undefined;

if (process.env.VITE_TLS_ENABLE === 'true') {
  // get current path
  const currentPath = __dirname;
  tls = {
    key: fs.readFileSync(`${currentPath}/../../.tls/key.pem`),
    cert: fs.readFileSync(`${currentPath}/../../.tls/cert.pem`),
  };
}

export default defineConfig(() => ({
  root: __dirname,
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
  plugins: [vue(), nxViteTsPaths(), nxCopyAssetsPlugin(['*.md'])],
  // Uncomment this if you are using workers.
  // worker: {
  //  plugins: [ nxViteTsPaths() ],
  // },
  build: {
    outDir: '../../dist/apps/front',
    emptyOutDir: true,
    reportCompressedSize: true,
    commonjsOptions: {
      transformMixedEsModules: true,
    },
  },
}));
