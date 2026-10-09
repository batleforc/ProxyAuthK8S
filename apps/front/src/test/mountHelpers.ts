import type { Directive } from 'vue';

/** `v-highlight` is registered globally by vue-hljs in main.ts; components under test get a no-op. */
export const stubDirectives: Record<string, Directive> = {
  highlight: () => undefined,
};
