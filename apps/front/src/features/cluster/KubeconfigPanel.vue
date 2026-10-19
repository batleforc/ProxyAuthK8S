<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { CodeBlock } from '@/components/ui/code-block';
import { downloadTextFile } from '../../utils/download.ts';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';
import { useToast } from '../../composables/useToast.ts';

/**
 * A kubeconfig and its two actions. Download is the view's one filled action;
 * copy is a control. The YAML stays text: nothing persists it.
 */
const props = defineProps<{
  content: string;
  filename: string;
  /** What the copy and download messages call it ("Kubeconfig"…). */
  name: string;
}>();

const { t } = useI18n();
const toast = useToast();
const copyToClipboard = useCopyToClipboard();

function download() {
  downloadTextFile(props.content, props.filename);
  toast.success(t('kubeconfig.downloaded', { name: props.name }));
}
</script>

<template>
  <CodeBlock
    :code="content"
    lang="yaml"
    :label="name"
  >
    <Button
      class="kubeconfig-download"
      @click="download"
    >
      {{ t('kubeconfig.download') }}
    </Button>
    <Button
      variant="outline"
      class="kubeconfig-copy"
      @click="copyToClipboard(content, name)"
    >
      {{ t('kubeconfig.copy') }}
    </Button>
  </CodeBlock>
</template>
