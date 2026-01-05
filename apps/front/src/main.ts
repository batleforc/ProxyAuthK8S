import router from './router/index.ts';
import type { Router } from 'vue-router';
import { createPinia } from 'pinia';
import { createApp, markRaw } from 'vue';
import App from './app/App.vue';
import { client } from '@proxy-auth-k8s/front-api';
import vueHljs from 'vue-hljs';
import hljs from 'highlight.js';

import { ToastOptions, ToastPlugin } from 'maz-ui/plugins/toast';
import { obsidian } from '@maz-ui/themes/presets/obsidian';
import { fr } from '@maz-ui/translations';
import { MazUi } from 'maz-ui/plugins/maz-ui';

import 'vue-hljs/style.css';
import 'maz-ui/styles';
import './styles.scss';

client.setConfig({
  // @ts-expect-error: VITE_API_BASE_URL is injected at build time
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
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

const toastOptions: ToastOptions = {
  position: 'bottom-right',
  timeout: 1000,
  persistent: false,
};

app.use(MazUi, {
  theme: {
    colorMode: 'dark',
    preset: obsidian,
    mode: 'dark',
  },
  translations: {
    messages: { fr },
  },
});

app.use(ToastPlugin, toastOptions);
app.use(vueHljs, { hljs });
app.use(pinia);
app.use(router);
app.mount('#root');
