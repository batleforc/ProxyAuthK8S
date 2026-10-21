<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { useAuthStore } from '../store/auth';
import { PageHeader } from '@/components/ui/page-header';
import { Button } from '@/components/ui/button';
import { CodeBlock } from '@/components/ui/code-block';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { useCopyToClipboard } from '../composables/useCopyToClipboard.ts';
import {
  backendUrlToName,
  buildUsageExamples,
  buildInstallationCommands,
  buildWorkflowExample,
  resolveBackendUrl,
} from '../utils/cliCommands.ts';

const { t } = useI18n();
const authStore = useAuthStore();
const copyToClipboard = useCopyToClipboard();

const backendUrl = computed(() => {
  const envUrl = import.meta.env.VITE_API_BASE_URL;
  return resolveBackendUrl(envUrl, globalThis.location.origin);
});

const backendUrlName = computed(() => {
  try {
    return backendUrlToName(backendUrl.value);
  } catch (error) {
    console.error('Error parsing backend URL:', error);
    return backendUrl.value;
  }
});

// Computed: `t` follows the locale, so the content follows a language switch.
const workflowExample = computed(() =>
  buildWorkflowExample(
    backendUrl.value,
    backendUrlName.value,
    authStore.getToken,
    t,
  ),
);

const installTabs = computed(() => {
  const commands = buildInstallationCommands(t);
  return (['mise', 'manual'] as const).map((key) => ({
    key,
    method: commands[key],
    code: commands[key].commands.join('\n'),
  }));
});
const usageSections = computed(() => {
  const examples = buildUsageExamples(
    backendUrl.value,
    backendUrlName.value,
    t,
  );
  return (['config', 'auth', 'clusters', 'contexts'] as const).map(
    (key) => examples[key],
  );
});
const resources = [
  {
    key: 'readme',
    url: 'https://github.com/batleforc/proxyauthk8s/tree/main/apps/kubectl_proxyauth',
  },
  {
    key: 'releases',
    url: 'https://github.com/batleforc/proxyauthk8s/releases',
  },
  { key: 'mise', url: 'https://mise.jdx.dev/dev-tools/backends/github.html' },
] as const;
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="poster"
      :title="t('cli.title')"
      :description="t('cli.description')"
    />
  </section>

  <ol class="m-0 list-none p-0">
    <li class="border-b border-rule-soft px-4 py-6 sm:px-6">
      <h2
        class="mb-4 mt-0 flex items-baseline gap-3 text-base font-normal text-foreground"
      >
        <span class="font-display text-sub text-muted-foreground">1</span
        >{{ t('cli.quickStart') }}
      </h2>
      <ol class="m-0 max-w-[68ch] pl-6 text-sm text-muted-foreground">
        <li
          v-for="step in ['install', 'configure', 'use']"
          :key="step"
          class="py-1"
        >
          {{ t(`cli.steps.${step}`) }}
        </li>
      </ol>
    </li>

    <li class="border-b border-rule-soft px-4 py-6 sm:px-6">
      <h2
        class="mb-4 mt-0 flex items-baseline gap-3 text-base font-normal text-foreground"
      >
        <span class="font-display text-sub text-muted-foreground">2</span
        >{{ t('cli.installation') }}
      </h2>
      <Tabs default-value="mise">
        <TabsList :aria-label="t('cli.installation')">
          <TabsTrigger
            v-for="tab in installTabs"
            :key="tab.key"
            :value="tab.key"
          >
            {{ tab.method.title }}
          </TabsTrigger>
        </TabsList>
        <TabsContent
          v-for="tab in installTabs"
          :key="tab.key"
          :value="tab.key"
        >
          <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
            <span
              class="mr-2 border border-border px-2 py-1 text-xs uppercase tracking-[0.1em] text-foreground"
              >{{ tab.method.badge }}</span
            >
            {{ tab.method.description }}
          </p>
          <CodeBlock
            :code="tab.code"
            lang="bash"
            :label="tab.method.title"
          >
            <Button
              variant="outline"
              size="sm"
              @click="copyToClipboard(tab.code, tab.method.title)"
            >
              {{ t('common.copy') }}
            </Button>
          </CodeBlock>
        </TabsContent>
      </Tabs>
    </li>

    <li class="border-b border-rule-soft px-4 py-6 sm:px-6">
      <h2
        class="mb-4 mt-0 flex items-baseline gap-3 text-base font-normal text-foreground"
      >
        <span class="font-display text-sub text-muted-foreground">3</span
        >{{ t('cli.usage') }}
      </h2>
      <details
        v-for="section in usageSections"
        :key="section.title"
        class="border-b border-dashed border-rule-soft py-3"
      >
        <summary class="cursor-pointer text-base text-foreground">
          {{ section.title }}
        </summary>
        <p class="mb-4 mt-2 max-w-[64ch] text-sm text-muted-foreground">
          {{ section.description }}
        </p>
        <div
          v-for="example in section.examples"
          :key="example.title"
          class="mb-5"
        >
          <h3 class="m-0 text-sm font-normal text-foreground">
            {{ example.title }}
          </h3>
          <p class="mb-2 mt-0 max-w-[64ch] text-sm text-muted-foreground">
            {{ example.description }}
          </p>
          <CodeBlock
            :code="example.command"
            lang="bash"
            :label="example.title"
          >
            <Button
              variant="outline"
              size="sm"
              @click="copyToClipboard(example.command, example.title)"
            >
              {{ t('common.copy') }}
            </Button>
          </CodeBlock>
        </div>
      </details>
    </li>

    <li class="border-b border-rule-soft px-4 py-6 sm:px-6">
      <h2
        class="mb-4 mt-0 flex items-baseline gap-3 text-base font-normal text-foreground"
      >
        <span class="font-display text-sub text-muted-foreground">4</span
        >{{ t('cli.workflow') }}
      </h2>
      <p class="mb-4 mt-0 max-w-[64ch] text-sm text-muted-foreground">
        {{ t('cli.workflowText') }}
      </p>
      <CodeBlock
        :code="workflowExample"
        lang="bash"
        :label="t('cli.workflow')"
      >
        <Button @click="copyToClipboard(workflowExample, t('cli.workflow'))">
          {{ t('cli.copyWorkflow') }}
        </Button>
      </CodeBlock>
    </li>
  </ol>

  <section
    class="px-4 py-6 sm:px-6"
    aria-labelledby="cli-resources"
  >
    <h2
      id="cli-resources"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('cli.resources') }}
    </h2>
    <ul class="m-0 list-none p-0">
      <li
        v-for="resource in resources"
        :key="resource.key"
        class="flex flex-wrap gap-x-4 border-b border-dashed border-rule-soft py-3"
      >
        <a
          :href="resource.url"
          target="_blank"
          rel="noopener noreferrer"
          class="text-base"
          >{{ t(`cli.links.${resource.key}.label`) }}</a
        >
        <span class="text-sm text-muted-foreground">{{
          t(`cli.links.${resource.key}.text`)
        }}</span>
      </li>
    </ul>
  </section>
</template>
