<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import MazAccordion from 'maz-ui/components/MazAccordion';
import {
  LazyMazClipboardDocument as MazClipboardDocument,
  LazyMazCodeBracket as MazCodeBracket,
  LazyMazCog6Tooth as MazCog6Tooth,
  LazyMazKey as MazKey,
  LazyMazCube as MazCube,
  LazyMazServer as MazServer
} from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';
import type { UsageExamples } from '../../utils/cliCommands.ts';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const props = defineProps<{
  usageExamples: UsageExamples;
}>();

const copyToClipboard = useCopyToClipboard();

// Accordion steps, in display order (MazAccordion slots are `title-N` / `content-N`, 1-based).
const sections = [
  { key: 'config', icon: MazCog6Tooth },
  { key: 'auth', icon: MazKey },
  { key: 'clusters', icon: MazServer },
  { key: 'contexts', icon: MazCube },
] as const;
</script>

<template>
  <SectionCard
    class="usage-card"
    heading-tag="h2"
    title="Guide d'Utilisation"
    :icon="MazCodeBracket"
  >
    <div class="usage-content">
      <MazAccordion class="usage-content-accordion">
        <template
          v-for="(section, index) in sections"
          :key="`title-${section.key}`"
          #[`title-${index+1}`]
        >
          <div class="accordion-title">
            <MazIcon
              :icon="section.icon"
              size="lg"
            />
            <span>{{ props.usageExamples[section.key].title }}</span>
          </div>
        </template>
        <template
          v-for="(section, index) in sections"
          :key="`content-${section.key}`"
          #[`content-${index+1}`]
        >
          <div class="accordion-content">
            <p class="section-description">
              {{ props.usageExamples[section.key].description }}
            </p>
            <div class="examples-list">
              <div
                v-for="example in props.usageExamples[section.key].examples"
                :key="example.title"
                class="example-item"
              >
                <h4 class="example-title">
                  {{ example.title }}
                </h4>
                <p class="example-description">
                  {{ example.description }}
                </p>
                <div class="example-command">
                  <div
                    v-highlight
                    class="example-code-container"
                  >
                    <pre><code class="hljs bash">{{ example.command }}</code></pre>
                    <MazBtn
                      size="xs"
                      color="primary"
                      :left-icon="MazClipboardDocument"
                      class="example-copy-btn"
                      @click="copyToClipboard(example.command, example.title)"
                    >
                      Copier
                    </MazBtn>
                  </div>
                </div>
              </div>
            </div>
          </div>
        </template>
      </MazAccordion>
    </div>
  </SectionCard>
</template>

<style scoped src="../../styles/hljs-overrides.css"></style>

<style scoped>
.usage-content-accordion {
  width: 100%;
}

.example-code-container {
  position: relative;
  border-radius: 6px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
}

.example-code-container pre {
  margin: 0;
  padding: 1rem;
  background: transparent;
  overflow-x: auto;
}

.example-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.75rem;
  background: transparent;
  color: #abb2bf;
}

.example-copy-btn {
  position: absolute;
  top: 0.5rem;
  right: 0.5rem;
  min-width: 50px;
  z-index: 10;
}

/* Usage Examples */
.usage-content {
  padding: 1rem 0;
}

/* Accordion Styling */
.usage-content .maz-accordion {
  width: 100%;
  max-width: 100%;
}

.usage-content .maz-accordion>div {
  width: 100%;
}

.accordion-title {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  color: #f1f5f9;
  font-weight: 500;
  width: 100%;
}

.accordion-content {
  padding: 1rem 0;
}

.section-description {
  color: #94a3b8;
  margin: 0 0 1.5rem 0;
  font-size: 0.875rem;
}

.examples-list {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.example-item {
  padding: 1rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 8px;
  border-left: 4px solid #3b82f6;
}

.example-title {
  font-size: 1rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 0.5rem 0;
}

.example-description {
  color: #94a3b8;
  margin: 0 0 1rem 0;
  font-size: 0.875rem;
}

.example-command {
  margin-top: 1rem;
}

@media (max-width: 768px) {
  .example-command {
    flex-direction: column;
    align-items: flex-start;
  }

  .example-copy-btn {
    align-self: flex-end;
  }
}
</style>
