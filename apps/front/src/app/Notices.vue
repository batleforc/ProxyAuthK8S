<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { dismissNotice, notices } from '../composables/useToast.ts';

/**
 * Where `useToast` messages land, replacing maz-ui's toasts. Two live regions,
 * always in the DOM so they announce: errors are assertive (`role="alert"`),
 * everything else polite (`role="status"`). Flat, framed, no shadow.
 */
const { t } = useI18n();
const errors = computed(() =>
  notices.filter((notice) => notice.kind === 'error'),
);
const others = computed(() =>
  notices.filter((notice) => notice.kind !== 'error'),
);
</script>

<template>
  <div
    class="pointer-events-none fixed inset-x-4 bottom-4 z-50 flex flex-col items-end gap-2"
  >
    <div
      v-for="region in [
        { role: 'alert', live: 'assertive', items: errors },
        { role: 'status', live: 'polite', items: others },
      ]"
      :key="region.role"
      :role="region.role"
      :aria-live="region.live"
      class="flex w-full max-w-[64ch] flex-col gap-2"
    >
      <p
        v-for="notice in region.items"
        :key="notice.id"
        class="pointer-events-auto m-0 flex items-start gap-3 border bg-ground-sunk px-4 py-3 text-sm"
        :class="
          notice.kind === 'error'
            ? 'border-destructive text-destructive'
            : 'border-border text-foreground'
        "
        :data-kind="notice.kind"
      >
        <span class="min-w-0 flex-1 [overflow-wrap:anywhere]">{{
          notice.message
        }}</span>
        <button
          type="button"
          class="cursor-pointer border-0 bg-transparent p-0 text-xs uppercase tracking-[0.1em] text-muted-foreground hover:text-foreground"
          :aria-label="t('notices.dismiss')"
          @click="dismissNotice(notice.id)"
        >
          ×
        </button>
      </p>
    </div>
  </div>
</template>
