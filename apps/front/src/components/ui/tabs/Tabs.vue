<!-- Ported from batlehub/ui/src/components/ui/tabs/Tabs.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { TabsRoot, type TabsRootProps } from "reka-ui";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsRootProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsRoot v-bind="props" :class="cn('', props.class)">
    <slot />
  </TabsRoot>
</template>
