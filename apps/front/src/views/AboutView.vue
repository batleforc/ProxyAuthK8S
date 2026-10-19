<script lang="ts" setup>
import { useI18n } from 'vue-i18n';
import { PageHeader } from '@/components/ui/page-header';

const { t } = useI18n();

const DOCS_URL = 'https://batleforc.github.io/ProxyAuthK8S/docs';
const REPO_URL = 'https://github.com/batleforc/ProxyAuthK8S';

// The API reference (Scalar) is served by the backend next to the API.
const apiBase = (import.meta.env.VITE_API_BASE_URL || '/api').replace(
  /\/api\/?$/,
  '',
);
const apiDocsUrl = `${apiBase}/api/docs`;

const features = ['entry', 'oidc', 'authz', 'kubectl'] as const;
const links = [
  { key: 'docs', href: DOCS_URL },
  { key: 'plugin', href: `${DOCS_URL}/kubectl-plugin` },
  // The architecture diagram that was on the landing page lives in the docs.
  { key: 'architecture', href: `${DOCS_URL}/architecture` },
  { key: 'api', href: apiDocsUrl },
  { key: 'source', href: REPO_URL },
] as const;
</script>

<template>
  <section class="border-b border-rule-soft px-4 pb-5 pt-10 sm:px-6">
    <PageHeader
      variant="poster"
      :title="t('about.title')"
      :description="t('about.lead')"
    />
  </section>

  <section
    class="border-b border-rule-soft px-4 py-6 sm:px-6"
    aria-labelledby="about-features"
  >
    <h2
      id="about-features"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('about.featuresTitle') }}
    </h2>
    <div
      v-for="feature in features"
      :key="feature"
      class="border-b border-dashed border-rule-soft py-3"
    >
      <h3 class="m-0 text-base font-normal text-foreground">
        {{ t(`about.features.${feature}.title`) }}
      </h3>
      <p class="m-0 max-w-[68ch] text-sub leading-[1.7] text-muted-foreground">
        {{ t(`about.features.${feature}.text`) }}
      </p>
    </div>
  </section>

  <section
    class="px-4 py-6 sm:px-6"
    aria-labelledby="about-links"
  >
    <h2
      id="about-links"
      class="mb-4 mt-0 text-xs font-normal uppercase tracking-[0.16em] text-muted-foreground"
    >
      {{ t('about.linksTitle') }}
    </h2>
    <ul class="m-0 list-none p-0">
      <li
        v-for="link in links"
        :key="link.key"
        class="flex flex-wrap gap-x-4 border-b border-dashed border-rule-soft py-3"
      >
        <a
          :href="link.href"
          target="_blank"
          rel="noopener noreferrer"
          class="text-base"
          >{{ t(`about.links.${link.key}.label`) }}</a
        >
        <span class="text-sm text-muted-foreground">{{
          t(`about.links.${link.key}.text`)
        }}</span>
      </li>
    </ul>
  </section>
</template>
