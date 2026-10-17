<script setup lang="ts">
import { computed } from 'vue';
import { useI18n } from 'vue-i18n';
import { Button } from '@/components/ui/button';
import { useAuthStore } from '../store/auth.ts';

const { t } = useI18n();
const authStore = useAuthStore();
const name = computed(
  () => authStore.getUserProfile?.profile?.name || t('user.fallback'),
);
</script>

<template>
  <div class="flex items-center gap-3">
    <template v-if="authStore.isAuthenticated">
      <p class="m-0 text-xs text-muted-foreground">
        <span class="sr-only">{{ t('user.signedInAs') }}</span>
        <b class="font-medium text-copper">{{ name }}</b>
      </p>
      <Button
        variant="outline"
        size="sm"
        @click="authStore.logOut()"
      >
        {{ t('user.logout') }}
      </Button>
    </template>
    <Button
      v-else
      variant="outline"
      size="sm"
      @click="authStore.logIn()"
    >
      {{ t('user.login') }}
    </Button>
  </div>
</template>
