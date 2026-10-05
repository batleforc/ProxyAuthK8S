<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import MazBadge from 'maz-ui/components/MazBadge';
import MazTabs from 'maz-ui/components/MazTabs';
import MazTabsBar from 'maz-ui/components/MazTabsBar';
import MazTabsContent from 'maz-ui/components/MazTabsContent';
import MazTabsContentItem from 'maz-ui/components/MazTabsContentItem';
import {
  LazyMazCloudArrowDown as MazCloudArrowDown,
  LazyMazClipboardDocument as MazClipboardDocument,
} from '@maz-ui/icons';
import SectionCard from '../common/SectionCard.vue';
import { installationCommands } from '../../utils/cliCommands.ts';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

const copyToClipboard = useCopyToClipboard();

// One tab per installation method, in display order.
const tabs = [
  { method: installationCommands.krew, badgeColor: 'success', copyLabel: 'Commandes Krew' },
  { method: installationCommands.manual, badgeColor: 'warning', copyLabel: 'Commandes manuelles' },
  { method: installationCommands.homebrew, badgeColor: 'info', copyLabel: 'Commandes Homebrew' },
] as const;
</script>

<template>
  <SectionCard
    class="installation-card"
    heading-tag="h2"
    title="Installation"
    :icon="MazCloudArrowDown"
  >
    <div class="installation-content">
      <MazTabs>
        <MazTabsBar
          :items="tabs.map(({ method }) => ({ label: method.title, disabled: false }))"
        />

        <MazTabsContent>
          <MazTabsContentItem
            v-for="(tab, index) in tabs"
            :key="tab.method.title"
            :tab="index + 1"
          >
            <div class="installation-tab">
              <div class="tab-header">
                <MazBadge
                  :color="tab.badgeColor"
                  size="sm"
                >
                  {{ tab.method.badge }}
                </MazBadge>
                <p class="tab-description">
                  {{ tab.method.description }}
                </p>
              </div>
              <div class="command-block">
                <div
                  v-highlight
                  class="code-container"
                >
                  <pre><code class="hljs bash">{{ tab.method.commands.join('\n') }}</code></pre>
                  <MazBtn
                    color="primary"
                    size="sm"
                    :left-icon="MazClipboardDocument"
                    class="copy-btn-overlay"
                    @click="copyToClipboard(tab.method.commands.join('\n'), tab.copyLabel)"
                  >
                    Copier
                  </MazBtn>
                </div>
              </div>
            </div>
          </MazTabsContentItem>
        </MazTabsContent>
      </MazTabs>
    </div>
  </SectionCard>
</template>

<style scoped src="../../styles/hljs-overrides.css"></style>

<style scoped>
.installation-content {
  padding: 1.5rem 0;
}

.installation-tab {
  padding: 1rem 0;
}

.tab-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1.5rem;
}

.tab-description {
  color: #94a3b8;
  margin: 0;
  font-size: 0.875rem;
}

.command-block {
  width: 100%;
}

/* Highlight.js Code Blocks */
.code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  width: 100%;
  max-width: 100%;
}

.code-container pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
}

.copy-btn-overlay {
  position: absolute;
  top: 0.75rem;
  right: 0.75rem;
  min-width: 60px;
  z-index: 10;
  opacity: 0.8;
  transition: opacity 0.2s ease;
}

.copy-btn-overlay:hover {
  opacity: 1;
}

@media (max-width: 768px) {
  .installation-tab {
    padding: 0.5rem 0;
  }

  .tab-header {
    flex-direction: column;
    align-items: flex-start;
    gap: 0.5rem;
    margin-bottom: 1rem;
  }

  .copy-btn-overlay {
    position: static;
    margin-top: 1rem;
    width: 100%;
  }

  .code-container {
    overflow-x: auto;
  }

  .code-container pre {
    padding: 1rem;
    font-size: 0.75rem;
  }
}

@media (max-width: 480px) {
  .installation-content {
    padding: 1rem 0;
  }

  .code-container pre code {
    font-size: 0.7rem;
  }

  .copy-btn-overlay {
    font-size: 0.75rem;
    padding: 0.5rem;
  }
}
</style>
