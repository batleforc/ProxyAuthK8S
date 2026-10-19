<!-- Ported from batlehub/ui/src/components/ui/table/TableHead.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <th
    :class="
      cn(
        'pb-2 pr-3 text-left align-bottom font-mono text-xs font-normal uppercase tracking-[0.14em] text-muted-foreground',
        props.class,
      )
    "
  >
    <slot />
  </th>
</template>
