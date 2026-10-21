<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { CodeBlock } from '@/components/ui/code-block';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

/** The three steps of a plugin-based setup, after a cluster callback. */
const props = defineProps<{ pluginCommands: string }>();

const { t } = useI18n();
const copyToClipboard = useCopyToClipboard();

const INSTALL = `# Via mise
mise use -g github:batleforc/ProxyAuthK8S

# Or a manual download
VERSION=$(curl -fsSLI -o /dev/null -w "%{url_effective}" https://github.com/batleforc/ProxyAuthK8S/releases/latest | sed "s|.*/||")
curl -fsSLO "https://github.com/batleforc/ProxyAuthK8S/releases/download/$VERSION/kubectl-proxyauth_\${VERSION}_linux_amd64.tar.gz"
tar xzf kubectl-proxyauth_*_linux_amd64.tar.gz
sudo install kubectl-proxyauth /usr/local/bin/`;
const VERIFY = `kubectl proxyauth get
kubectl proxyauth ctx
kubectl get nodes`;

const steps = [
  { key: 'install', code: INSTALL },
  { key: 'connect', code: props.pluginCommands },
  { key: 'verify', code: VERIFY },
] as const;
</script>

<template>
  <ol class="m-0 flex list-none flex-col gap-6 p-0">
    <li
      v-for="(step, index) in steps"
      :key="step.key"
    >
      <h4
        class="mb-3 mt-0 flex items-baseline gap-3 text-base font-normal text-foreground"
      >
        <span class="font-display text-sub text-muted-foreground">{{
          index + 1
        }}</span>
        {{ t(`pluginSteps.${step.key}`) }}
      </h4>
      <CodeBlock
        :code="step.code"
        lang="bash"
        :label="t(`pluginSteps.${step.key}`)"
      >
        <Button
          variant="outline"
          size="sm"
          @click="copyToClipboard(step.code, t(`pluginSteps.${step.key}`))"
        >
          {{ t('common.copy') }}
        </Button>
      </CodeBlock>
    </li>
  </ol>
</template>
