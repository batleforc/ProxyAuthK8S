<script setup lang="ts">
import { RouterLink } from 'vue-router';
import { useI18n } from 'vue-i18n';
import { useAuthStore } from '../store/auth.ts';
import { PageHeader } from '@/components/ui/page-header';
import { Button } from '@/components/ui/button';

const { t } = useI18n();
const authStore = useAuthStore();

const features = [
  'security',
  'unified',
  'instant',
  'teams',
  'audit',
  'flexible',
] as const;
const steps = ['login', 'explore', 'access', 'use'] as const;
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="poster"
      title="ProxyAuthK8S"
      :description="t('landing.tagline')"
    />
    <div class="mt-5">
      <Button @click="authStore.logIn()">
        {{ t('landing.login') }}
      </Button>
    </div>
  </section>

  <section
    class="border-b border-rule-soft px-4 py-6 sm:px-6"
    aria-labelledby="why"
  >
    <h2
      id="why"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('landing.why') }}
    </h2>
    <dl class="m-0">
      <div
        v-for="feature in features"
        :key="feature"
        class="grid gap-1 border-b border-dashed border-rule-soft py-3 sm:grid-cols-[minmax(0,16rem)_minmax(0,1fr)] sm:gap-4"
      >
        <dt class="text-base text-foreground">
          {{ t(`landing.features.${feature}.title`) }}
        </dt>
        <dd class="m-0 max-w-[68ch] text-sm text-muted-foreground">
          {{ t(`landing.features.${feature}.text`) }}
        </dd>
      </div>
    </dl>
  </section>

  <section
    class="border-b border-rule-soft px-4 py-6 sm:px-6"
    aria-labelledby="how"
  >
    <h2
      id="how"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('landing.how') }}
    </h2>
    <ol class="m-0 list-none p-0">
      <li
        v-for="(step, index) in steps"
        :key="step"
        class="flex gap-4 border-b border-dashed border-rule-soft py-3"
      >
        <span
          class="font-display text-sub text-muted-foreground"
          aria-hidden="true"
          >{{ index + 1 }}</span
        >
        <div class="min-w-0">
          <p class="m-0 text-base text-foreground">
            {{ t(`landing.steps.${step}.title`) }}
          </p>
          <p class="m-0 max-w-[68ch] text-sm text-muted-foreground">
            {{ t(`landing.steps.${step}.text`) }}
          </p>
        </div>
      </li>
    </ol>
  </section>

  <section class="px-4 py-6 sm:px-6">
    <p class="m-0 max-w-[68ch] text-sm text-muted-foreground">
      {{ t('landing.more') }}
      <RouterLink to="/about">
        {{ t('landing.aboutLink') }}
      </RouterLink>
    </p>
  </section>
</template>
