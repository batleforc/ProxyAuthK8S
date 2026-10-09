<!-- Centered section heading shared by the landing page sections. -->
<template>
  <h2 class="section-title">
    <slot />
  </h2>
</template>

<style scoped>
.section-title {
  font-size: 2.5rem;
  font-weight: 700;
  text-align: center;
  margin: 0 0 4rem 0;
  color: #1e293b;
  letter-spacing: -0.025em;
}

@media (max-width: 768px) {
  .section-title {
    font-size: 2rem;
  }
}
</style>
