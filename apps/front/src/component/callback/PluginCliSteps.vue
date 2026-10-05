<script setup lang="ts">
import MazBtn from 'maz-ui/components/MazBtn';
import MazIcon from 'maz-ui/components/MazIcon';
import {
  LazyMazClipboardDocument,
  LazyMazCloudArrowDown,
  LazyMazCog6Tooth,
  LazyMazShieldCheck,
} from '@maz-ui/icons';
import { useCopyToClipboard } from '../../composables/useCopyToClipboard.ts';

/** "Configuration via Plugin CLI" tab: plugin advantages and the 3 setup steps. */
defineProps<{
  pluginCommands: string;
}>();

const copyToClipboard = useCopyToClipboard();
</script>

<template>
  <div class="cli-steps">
    <div class="cli-advantages">
      <h4 class="advantages-title">
        Avantages du plugin CLI:
      </h4>
      <ul class="advantages-list">
        <li>🔄 Gestion automatique du renouvellement des tokens</li>
        <li>🚀 Intégration native avec kubectl</li>
        <li>⚙️ Configuration centralisée des clusters</li>
        <li>🔐 Authentification sécurisée via navigateur</li>
        <li>📋 Gestion des contextes kubectl simplifiée</li>
      </ul>
    </div>
    <div class="cli-step">
      <h4 class="cli-step-title">
        <MazIcon
          :icon="LazyMazCloudArrowDown"
          size="sm"
        />
        Étape 1: Installer le plugin (si pas déjà fait)
      </h4>
      <div
        v-highlight
        class="cli-code-container"
      >
        <pre><code class="hljs bash"># Via Krew (recommandé)
kubectl krew install proxyauth

# Ou téléchargement manuel
wget https://github.com/batleforc/proxyauthk8s/releases/latest/download/kubectl-proxyauth-linux-amd64
chmod +x kubectl-proxyauth-linux-amd64
sudo mv kubectl-proxyauth-linux-amd64 /usr/local/bin/kubectl-proxyauth</code></pre>
        <MazBtn
          size="sm"
          color="primary"
          :left-icon="LazyMazClipboardDocument"
          class="cli-copy-btn"
          @click="copyToClipboard('kubectl krew install proxyauth', 'Commande d\'installation')"
        >
          Copier installation
        </MazBtn>
      </div>
    </div>

    <div class="cli-step">
      <h4 class="cli-step-title">
        <MazIcon
          :icon="LazyMazCog6Tooth"
          size="sm"
        />
        Étape 2: Configurer et se connecter
      </h4>
      <div
        v-highlight
        class="cli-code-container"
      >
        <pre><code class="hljs bash">{{ pluginCommands }}</code></pre>
        <MazBtn
          size="sm"
          color="primary"
          :left-icon="LazyMazClipboardDocument"
          class="cli-copy-btn"
          @click="copyToClipboard(pluginCommands, 'Commandes CLI')"
        >
          Copier commandes
        </MazBtn>
      </div>
    </div>

    <div class="cli-step">
      <h4 class="cli-step-title">
        <MazIcon
          :icon="LazyMazShieldCheck"
          size="sm"
        />
        Étape 3: Vérifier la configuration
      </h4>
      <div
        v-highlight
        class="cli-code-container"
      >
        <pre><code class="hljs bash"># Vérifier la liste des clusters disponibles
kubectl proxyauth get

# Vérifier le contexte actuel
kubectl proxyauth ctx

# Tester la connexion
kubectl get nodes</code></pre>
        <MazBtn
          size="sm"
          color="primary"
          :left-icon="LazyMazClipboardDocument"
          class="cli-copy-btn"
          @click="copyToClipboard('kubectl proxyauth get\nkubectl proxyauth ctx\nkubectl get nodes', 'Commandes de vérification')"
        >
          Copier vérification
        </MazBtn>
      </div>
    </div>
  </div>
</template>

<!-- Every `code.hljs` of this component sits in a `.cli-code-container`. -->
<style scoped src="../../styles/hljs-overrides.css"></style>

<style scoped>
/* CLI Steps */
.cli-steps {
  display: flex;
  flex-direction: column;
  gap: 2rem;
  margin-bottom: 2rem;
}

.cli-step {
  padding: 1.5rem;
  background: rgba(255, 255, 255, 0.05);
  border-radius: 12px;
  border-left: 4px solid #3b82f6;
}

.cli-step-title {
  font-size: 1.125rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.cli-code-container {
  position: relative;
  border-radius: 8px;
  overflow: hidden;
  background: #282c34;
  border: 1px solid rgba(255, 255, 255, 0.1);
  margin-bottom: 1rem;
}

.cli-code-container pre {
  margin: 0;
  padding: 1.5rem;
  background: transparent;
  overflow-x: auto;
  line-height: 1.5;
}

.cli-code-container pre code {
  font-family: 'Monaco', 'Menlo', 'Ubuntu Mono', monospace;
  font-size: 0.875rem;
  background: transparent;
  color: #abb2bf;
}

.cli-copy-btn {
  position: absolute;
  top: 0.75rem;
  right: 0.75rem;
  min-width: 120px;
  z-index: 10;
  opacity: 0.8;
  transition: opacity 0.2s ease;
}

.cli-copy-btn:hover {
  opacity: 1;
}

/* CLI Advantages */
.cli-advantages {
  padding: 1.5rem;
  background: rgba(34, 197, 94, 0.1);
  border-radius: 12px;
  border-left: 4px solid #22c55e;
}

.advantages-title {
  font-size: 1.125rem;
  font-weight: 600;
  color: #f1f5f9;
  margin: 0 0 1rem 0;
}

.advantages-list {
  list-style: none;
  padding: 0;
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.advantages-list li {
  color: #cbd5e1;
  font-size: 0.875rem;
  line-height: 1.5;
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

@media (max-width: 768px) {
  .cli-copy-btn {
    position: static;
    margin-top: 1rem;
    width: 100%;
  }

  .cli-step {
    padding: 1rem;
  }

  .cli-steps {
    gap: 1.5rem;
  }
}
</style>
