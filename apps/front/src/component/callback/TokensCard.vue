<script setup lang="ts">
import { computed } from 'vue';
import MazBtn from 'maz-ui/components/MazBtn';
import MazTextarea from 'maz-ui/components/MazTextarea';
import { LazyMazClipboardDocument, LazyMazKey } from '@maz-ui/icons';
import type { CallbackModel } from '@proxy-auth-k8s/front-api';
import SectionCard from '../common/SectionCard.vue';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const props = defineProps<{
  tokens: CallbackModel;
}>();

const copyToClipboard = useCopyToClipboard();

// The refresh token is only shown when the provider returned one.
const fields = computed(() => [
  { id: 'access-token', label: 'Access Token', value: props.tokens.access_token },
  { id: 'id-token', label: 'ID Token', value: props.tokens.id_token },
  ...(props.tokens.refresh_token !== ''
    ? [{ id: 'refresh-token', label: 'Refresh Token', value: props.tokens.refresh_token }]
    : []),
]);
</script>

<template>
  <SectionCard
    class="tokens-card"
    title="Tokens d'Authentification"
    :icon="LazyMazKey"
  >
    <div class="tokens-content">
      <div
        v-for="field in fields"
        :key="field.id"
        class="token-section"
      >
        <label
          :for="field.id"
          class="token-label"
        >{{ field.label }}:</label>
        <div class="token-field">
          <MazTextarea
            :id="field.id"
            :model-value="field.value"
            readonly
            :rows="3"
            class="token-textarea"
          />
          <MazBtn
            size="sm"
            color="primary"
            :left-icon="LazyMazClipboardDocument"
            class="copy-button"
            @click="copyToClipboard(field.value, field.label)"
          >
            Copier
          </MazBtn>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped>
.tokens-content {
  padding: 1rem 0;
}

.token-section {
  margin-bottom: 1.5rem;
}

.token-label {
  display: block;
  font-weight: 500;
  color: #cbd5e1;
  margin-bottom: 0.5rem;
}

.token-field {
  display: flex;
  gap: 0.5rem;
  align-items: flex-start;
}

.token-textarea {
  flex: 1;
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
}

.copy-button {
  min-width: 80px;
}

@media (max-width: 768px) {
  .token-field {
    flex-direction: column;
  }

  .copy-button {
    align-self: flex-start;
  }
}
</style>
