<!-- Ported from batlehub/ui/src/components/ui/table/TableHeader.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <thead :class="cn('[&_tr]:border-b [&_tr]:border-solid [&_tr]:border-border [&_tr]:hover:bg-transparent', props.class)">
    <slot />
  </thead>
</template>
