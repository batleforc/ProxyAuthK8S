<!-- Ported from batlehub/ui/src/components/ui/table/TableRow.vue (batlehub commit 3f18ab70,
     Apache-2.0). radix-vue imports moved to reka-ui; `task ui:diff-batlehub`
     compares this file to its source. -->
<script setup lang="ts">
import { type HTMLAttributes } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{ class?: HTMLAttributes["class"] }>();
</script>

<template>
  <tr
    :class="
      cn(
        // Dashed at rest, solid under state (DESIGN.md): the hover is the raised
        // fill *and* a solid strong divider, since the fill alone is invisible.
        'border-b border-dashed border-rule-soft hover:border-solid hover:border-border hover:bg-accent data-[state=selected]:bg-accent',
        props.class,
      )
    "
  >
    <slot />
  </tr>
</template>
