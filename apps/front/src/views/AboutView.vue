<script lang="ts" setup>
import MazCard from 'maz-ui/components/MazCard';
import MazBtn from 'maz-ui/components/MazBtn';

const DOCS_URL = 'https://batleforc.github.io/ProxyAuthK8S/docs';
const REPO_URL = 'https://github.com/batleforc/ProxyAuthK8S';

// The API reference (Scalar) is served by the backend next to the API.
const apiBase = (import.meta.env.VITE_API_BASE_URL || '/api').replace(/\/api\/?$/, '');
const apiDocsUrl = `${apiBase}/api/docs`;

const features = [
  {
    title: 'Un point d’entrée unique',
    text: 'Chaque cluster est exposé derrière ProxyAuthK8S : une seule URL, un seul fournisseur d’identité.',
  },
  {
    title: 'Authentification OIDC',
    text: 'Connexion via votre IdP, validation des tokens (userinfo, audience, introspection) avant tout appel au cluster.',
  },
  {
    title: 'Autorisation fine',
    text: 'Groupes autorisés par cluster, ressources et namespaces filtrés, quotas, bannissement et journal d’audit.',
  },
  {
    title: 'kubectl natif',
    text: 'Le plugin kubectl proxyauth gère la connexion, le trousseau et le kubeconfig à votre place.',
  },
];

const links = [
  { label: 'Documentation', href: `${DOCS_URL}`, description: 'Installation, configuration, sécurité' },
  { label: 'Plugin kubectl', href: `${DOCS_URL}/kubectl-plugin`, description: 'Installer et utiliser kubectl proxyauth' },
  { label: 'Référence API', href: apiDocsUrl, description: 'API interactive de ce serveur' },
  { label: 'Code source', href: REPO_URL, description: 'GitHub — issues et contributions' },
];
</script>

<template>
  <main class="about">
    <section class="about-hero">
      <h1>À propos de ProxyAuthK8S</h1>
      <p class="about-lead">
        ProxyAuthK8S est un reverse proxy authentifiant pour Kubernetes : il centralise l’accès à vos
        clusters derrière votre fournisseur OIDC et applique vos règles d’autorisation à chaque requête.
      </p>
    </section>

    <section class="about-grid">
      <MazCard v-for="feature in features" :key="feature.title" class="about-card" elevation>
        <h2>{{ feature.title }}</h2>
        <p>{{ feature.text }}</p>
      </MazCard>
    </section>

    <section class="about-links">
      <h2>Aller plus loin</h2>
      <ul>
        <li v-for="link in links" :key="link.label">
          <a :href="link.href" target="_blank" rel="noopener noreferrer">
            <MazBtn color="transparent" outlined>{{ link.label }}</MazBtn>
          </a>
          <span class="about-link-description">{{ link.description }}</span>
        </li>
      </ul>
    </section>
  </main>
</template>

<style scoped>
.about {
  max-width: 64rem;
  margin: 0 auto;
  padding: 2rem 1rem 3rem;
  display: flex;
  flex-direction: column;
  gap: 2rem;
}

.about-hero h1 {
  font-size: 2rem;
  margin: 0 0 0.75rem;
}

.about-lead {
  font-size: 1.125rem;
  line-height: 1.6;
  opacity: 0.85;
  margin: 0;
}

.about-grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(14rem, 1fr));
  gap: 1rem;
}

.about-card h2 {
  font-size: 1.1rem;
  margin: 0 0 0.5rem;
}

.about-card p {
  margin: 0;
  line-height: 1.5;
  opacity: 0.85;
}

.about-links h2 {
  font-size: 1.25rem;
  margin: 0 0 1rem;
}

.about-links ul {
  list-style: none;
  margin: 0;
  padding: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.about-links li {
  display: flex;
  align-items: center;
  gap: 1rem;
  flex-wrap: wrap;
}

.about-link-description {
  opacity: 0.75;
}
</style>
