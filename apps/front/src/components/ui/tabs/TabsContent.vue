<!-- Ported from batlehub/ui/src/components/ui/tabs/TabsContent.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { TabsContent, type TabsContentProps } from "reka-ui";
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<TabsContentProps & { class?: HTMLAttributes["class"] }>();
</script>

<template>
  <TabsContent
    v-bind="props"
    :class="
      cn(
        'mt-4',
        props.class,
      )
    "
  >
    <slot />
  </TabsContent>
</template>
