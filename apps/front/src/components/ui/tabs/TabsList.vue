<!-- Ported from batlehub/ui/src/components/ui/tabs/TabsList.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { TabsList, type TabsListProps } from "reka-ui";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsListProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsList
    v-bind="props"
    :class="
      cn(
        // DESIGN.md's segment grammar: one 1px frame, 1px cell dividers, no gap.
        'flex flex-wrap border border-border text-muted-foreground',
        props.class,
      )
    "
  >
    <slot />
  </TabsList>
</template>
