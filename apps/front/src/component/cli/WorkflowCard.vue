<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import {
  LazyMazCommandLine as MazCommandLine,
  LazyMazClipboardDocument as MazClipboardDocument,
} from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

defineProps<{
  workflowExample: string;
}>();

const copyToClipboard = useCopyToClipboard();
</script>

<template>
  <SectionCard
    class="workflow-card"
    heading-tag="h2"
    title="Workflow Complet"
    :icon="MazCommandLine"
  >
    <div class="workflow-content">
      <p class="workflow-description">
        Exemple complet d'utilisation du plugin de l'installation à l'utilisation quotidienne.
      </p>
      <div class="workflow-command-block">
        <div
          v-highlight
          class="workflow-code-container"
        >
          <pre><code class="hljs bash">{{ workflowExample }}</code></pre>
          <MazBtn
            color="success"
            size="lg"
            :left-icon="MazClipboardDocument"
            class="workflow-copy-btn-overlay"
            @click="copyToClipboard(workflowExample, 'Workflow complet')"
          >
            Copier le workflow complet
          </MazBtn>
        </div>
      </div>
    </div>
  </SectionCard>
</template>

<style scoped src="../../styles/hljs-overrides.css"></style>

<style scoped>
.workflow-code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.workflow-code-container pre {
  margin: 0;
  padding: 2rem;
  background: transparent;
  overflow-x: auto;
  max-height: 400px;
  overflow-y: auto;
}

.workflow-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
  line-height: 1.6;
}

.workflow-copy-btn-overlay {
  position: absolute;
  top: 1rem;
  right: 1rem;
  min-width: 200px;
  z-index: 10;
  opacity: 0.9;
  transition: opacity 0.2s ease;
}

.workflow-copy-btn-overlay:hover {
  opacity: 1;
}

/* Workflow */
.workflow-content {
  padding: 1.5rem 0;
}

.workflow-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  font-size: 1rem;
  line-height: 1.6;
}

.workflow-command-block {
  margin-top: 1rem;
}
</style>
