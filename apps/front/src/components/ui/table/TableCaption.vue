<!-- Ported from batlehub/ui/src/components/ui/table/TableCaption.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <caption :class="cn('pb-3 text-left text-xs uppercase tracking-[0.1em] text-muted-foreground', props.class)">
    <slot />
  </caption>
</template>
