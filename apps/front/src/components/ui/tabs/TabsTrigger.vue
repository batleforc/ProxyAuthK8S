<!-- Ported from batlehub/ui/src/components/ui/tabs/TabsTrigger.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { TabsTrigger, type TabsTriggerProps } from "reka-ui";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsTriggerProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsTrigger
    v-bind="props"
    :class="
      cn(
        // A segment cell: Meta uppercase, the chosen one reversed to an ink block
        // like the current nav cell. The shared amber ring is the focus.
        'border-r border-border px-3 py-2 text-xs uppercase tracking-[0.06em] last:border-r-0 hover:text-foreground disabled:pointer-events-none disabled:opacity-50 data-[state=active]:bg-foreground data-[state=active]:font-bold data-[state=active]:text-background',
        props.class,
      )
    "
  >
    <slot />
  </TabsTrigger>
</template>
