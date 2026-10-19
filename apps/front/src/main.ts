import router from './router/index.ts';
import type { Router } from 'vue-router';
import { createPinia } from 'pinia';
import { createApp, markRaw, watch } from 'vue';
import App from './app/App.vue';
import { client } from '@proxy-auth-k8s/front-api';
import { i18n } from './i18n';
import { installStaleChunkReload } from './utils/staleChunk.ts';
import {
  installPreferences,
  resolvedLocale,
} from './composables/usePreferences.ts';

import './assets/index.css';

installStaleChunkReload();

client.setConfig({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
});

installPreferences();
watch(resolvedLocale, (locale) => {
  i18n.global.locale.value = locale;
});

const pinia = createPinia();
const app = createApp(App);

declare const RawSymbol: unique symbol;
declare module 'pinia' {
  export interface PiniaCustomProperties {
    // by using a setter we can allow both strings and refs
    router: Router & { [RawSymbol]?: true };
    language: string;
  }
}
pinia.use(({ store }) => {
  store.router = markRaw(router);
});

app.use(i18n);
app.use(pinia);
app.use(router);
app.mount('#root');
