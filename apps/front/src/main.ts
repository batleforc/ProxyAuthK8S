import router from './router/index.ts';
import type { Router } from 'vue-router';
import { createPinia } from 'pinia';
import { createApp, markRaw } from 'vue';
import App from './app/App.vue';
import { client } from '@proxy-auth-k8s/front-api';

import { obsidian } from '@maz-ui/themes/presets/obsidian';
import { fr } from '@maz-ui/translations';
import { MazUi } from 'maz-ui/plugins/maz-ui';

import 'maz-ui/styles';
import './styles.scss';

client.setConfig({
  baseURL: import.meta.env.VITE_API_BASE_URL || '/api',
});

const pinia = createPinia();
const app = createApp(App);

declare const RawSymbol: unique symbol;
declare module 'pinia' {
  export interface PiniaCustomProperties {
    // by using a setter we can allow both strings and refs
    router: Router & { [RawSymbol]?: true | undefined };
    language: string;
  }
}
pinia.use(({ store }) => {
  store.router = markRaw(router);
});

app.use(MazUi, {
  theme: {
    preset: obsidian,
  },
  translations: {
    messages: { fr },
  },
});

app.use(pinia);
app.use(router);
app.mount('#root');
