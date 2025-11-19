import './styles.scss';
import router from './router';
import type { Router } from 'vue-router';
import { createPinia } from 'pinia';
import { createApp, markRaw } from 'vue';
import App from './app/App.vue';
import {} from '@proxy-auth-k8s/front-api';

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

app.use(pinia);
app.use(router);
app.mount('#root');
