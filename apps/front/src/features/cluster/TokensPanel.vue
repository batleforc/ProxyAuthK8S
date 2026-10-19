<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import type { CallbackModel } from '@proxy-auth-k8s/front-api';
import { Button } from '@/components/ui/button';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const props = defineProps<{ tokens: CallbackModel }>();

const { t } = useI18n();
const copyToClipboard = useCopyToClipboard();

// The refresh token is only shown when the provider returned one.
const fields = computed(() => [
  {
    id: 'access-token',
    label: 'Access Token',
    value: props.tokens.access_token,
  },
  { id: 'id-token', label: 'ID Token', value: props.tokens.id_token },
  ...(props.tokens.refresh_token !== ''
    ? [
        {
          id: 'refresh-token',
          label: 'Refresh Token',
          value: props.tokens.refresh_token,
        },
      ]
    : []),
]);
</script>

<template>
  <div class="flex flex-col gap-5">
    <div
      v-for="field in fields"
      :key="field.id"
      class="token-section"
    >
      <label
        :for="field.id"
        class="token-label mb-2 block text-xs uppercase tracking-[0.14em] text-muted-foreground"
        >{{ field.label }}</label
      >
      <textarea
        :id="field.id"
        :value="field.value"
        readonly
        rows="3"
        class="block w-full resize-y border border-border bg-ground-sunk p-3 font-mono text-sm text-foreground [overflow-wrap:anywhere]"
      />
      <Button
        variant="outline"
        size="sm"
        class="token-copy mt-2"
        @click="copyToClipboard(field.value, field.label)"
      >
        {{ t('tokens.copy', { label: field.label }) }}
      </Button>
    </div>
  </div>
</template>
