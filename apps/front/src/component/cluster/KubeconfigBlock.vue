<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import { LazyMazClipboardDocument, LazyMazCloudArrowDown } from '@maz-ui/icons';

/** Highlighted kubeconfig YAML followed by "download" and "copy" buttons. */
withDefaults(
  defineProps<{
    content: string;
    downloadLabel: string;
    copyLabel: string;
    /** Dark bordered, scrollable code frame (NoSSO page) instead of the bare `<pre>`. */
    framed?: boolean;
  }>(),
  { framed: false },
);

const emit = defineEmits<{
  download: [];
  copy: [];
}>();
</script>

<template>
  <div
    v-highlight
    class="kubeconfig-field"
    :class="{ 'kubeconfig-field--framed': framed }"
  >
    <pre><code class="hljs yaml">{{ content }}</code></pre>
  </div>

  <div class="kubeconfig-actions">
    <MazBtn
      color="success"
      size="lg"
      :left-icon="LazyMazCloudArrowDown"
      class="download-button"
      @click="emit('download')"
    >
      {{ downloadLabel }}
    </MazBtn>
    <MazBtn
      color="primary"
      size="lg"
      :left-icon="LazyMazClipboardDocument"
      class="copy-kubeconfig-button"
      @click="emit('copy')"
    >
      {{ copyLabel }}
    </MazBtn>
  </div>
</template>

<style scoped>
.kubeconfig-field {
  margin-bottom: 1.5rem;
}

.kubeconfig-field--framed {
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  max-height: 400px;
  overflow-y: auto;
}

.kubeconfig-field--framed pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.kubeconfig-field--framed pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
  background: transparent;
  color: #abb2bf;
}

.kubeconfig-actions {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.download-button,
.copy-kubeconfig-button {
  min-width: 180px;
}

@media (max-width: 768px) {
  .kubeconfig-actions {
    flex-direction: column;
  }

  .download-button,
  .copy-kubeconfig-button {
    width: 100%;
  }
}
</style>
