<script setup lang="ts">
import HeroSection from '../component/home/HeroSection.vue';
import FeatureGrid from '../component/home/FeatureGrid.vue';
import HowItWorks from '../component/home/HowItWorks.vue';
import CtaSection from '../component/home/CtaSection.vue';
</script>

<template>
  <div class="home-container">
    <HeroSection />
    <FeatureGrid />
    <HowItWorks />
    <CtaSection />
  </div>
</template>

<style scoped>
.home-container {
  min-height: 100vh;
  font-family: var(--font-family-sans, 'Inter', sans-serif);
}
</style>
