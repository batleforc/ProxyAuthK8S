<!-- Ported from batlehub/ui/src/components/ui/table/TableCell.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <td :class="cn('py-3 pr-3 align-baseline', props.class)">
    <slot />
  </td>
</template>
