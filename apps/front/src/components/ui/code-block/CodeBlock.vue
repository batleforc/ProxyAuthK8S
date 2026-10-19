<script setup lang="ts">
/**
 * A command or a kubeconfig, set as text. Written for this front rather than
 * ported from batlehub's shiki block (RFC 0002 §17): ink on the sunk ground
 * keeps the One Synthetic Rule (crimson is the only invented colour), needs no
 * `v-html` and no inline style, so the CSP stays `style-src 'self'`.
 *
 * The block scrolls inside itself, never the body, and is focusable so the
 * scroll is reachable by keyboard.
 */
withDefaults(defineProps<{ code: string; label: string; lang?: string }>(), {
  lang: 'text',
});
</script>

<template>
  <div class="min-w-0">
    <pre
      class="m-0 overflow-x-auto overscroll-x-contain border border-border bg-ground-sunk p-4 text-sm leading-relaxed text-foreground"
      tabindex="0"
      role="region"
      :aria-label="label"
      :data-lang="lang"
    ><code>{{ code }}</code></pre>
    <div
      v-if="$slots.default"
      class="mt-3 flex flex-wrap gap-3"
    >
      <slot />
    </div>
  </div>
</template>
