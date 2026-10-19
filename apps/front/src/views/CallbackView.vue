<script setup lang="ts">
import { useI18n } from 'vue-i18n';
import { PageHeader } from '@/components/ui/page-header';
import { Resolution } from '@/components/ui/resolution';
import { Skeleton } from '@/components/ui/skeleton';

// The store finishes the OIDC callback (store/auth.ts) and navigates away:
// this view only says what is happening meanwhile.
const { t } = useI18n();
</script>

<template>
  <section
    class="px-4 py-10 sm:px-6"
    aria-busy="true"
  >
    <PageHeader
      variant="display"
      :title="t('callback.title')"
      :description="t('callback.text')"
    />
    <p class="mt-5">
      <Resolution
        state="pending"
        :label="t('callback.state')"
      />
    </p>
    <Skeleton
      class="mt-4 max-w-[64ch]"
      :lines="2"
    />
  </section>
</template>
