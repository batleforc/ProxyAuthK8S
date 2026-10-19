<script setup lang="ts">
import { nextTick, onBeforeUnmount, ref, useId, watch } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import ThemeToggle from './ThemeToggle.vue';
import LocaleToggle from './LocaleToggle.vue';

/**
 * The settings popover (DESIGN.md "Preferences"): a 264px box on the sunk
 * ground in a 1px frame, anchored to the right edge of its trigger, 8px below,
 * floated by z-index alone. Escape closes it and returns focus to the trigger,
 * a click outside dismisses it, opening focuses the first cell. Written by
 * hand: a headless popover would bring floating-ui into the first load.
 */
const { t } = useI18n();
const open = ref(false);
const id = useId();
const root = ref<HTMLElement | null>(null);
const panel = ref<HTMLElement | null>(null);

function trigger() {
  return root.value?.querySelector<HTMLElement>('[aria-controls]') ?? null;
}

function close(returnFocus: boolean) {
  open.value = false;
  if (returnFocus) trigger()?.focus();
}

function onKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && open.value) close(true);
}

function onPointerDown(event: PointerEvent) {
  if (open.value && !root.value?.contains(event.target as Node)) close(false);
}

function unlisten() {
  document.removeEventListener('keydown', onKeydown);
  document.removeEventListener('pointerdown', onPointerDown);
}

watch(open, async (value) => {
  if (value) {
    document.addEventListener('keydown', onKeydown);
    document.addEventListener('pointerdown', onPointerDown);
    await nextTick();
    panel.value?.querySelector<HTMLElement>('button')?.focus();
  } else {
    unlisten();
  }
});
// Directly: the watcher above is already stopped when the component unmounts.
onBeforeUnmount(unlisten);
</script>

<template>
  <div
    ref="root"
    class="relative"
  >
    <Button
      variant="outline"
      size="sm"
      :aria-expanded="open"
      :aria-controls="id"
      @click="open = !open"
    >
      {{ t('prefs.open') }}
    </Button>
    <section
      v-if="open"
      :id="id"
      ref="panel"
      :aria-label="t('prefs.title')"
      class="absolute right-0 top-[calc(100%+var(--s2))] z-50 w-[264px] max-w-[calc(100vw-2rem)] border border-border bg-ground-sunk p-4 text-foreground"
    >
      <h2 class="m-0 font-display text-sub uppercase tracking-[0.04em]">
        {{ t('prefs.title') }}
      </h2>
      <ThemeToggle />
      <LocaleToggle />
    </section>
  </div>
</template>
