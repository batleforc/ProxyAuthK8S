<template>
  <div class="callback-container">
    <div class="callback-content">
      <!-- Logo/Brand -->
      <div class="brand-section">
        <div class="brand-icon">
          <svg viewBox="0 0 24 24" fill="none" xmlns="http://www.w3.org/2000/svg">
            <path d="M12 2L2 7L12 12L22 7L12 2Z" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            <path d="M2 17L12 22L22 17" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
            <path d="M2 12L12 17L22 12" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"/>
          </svg>
        </div>
        <h1 class="brand-text">ProxyAuthK8S</h1>
      </div>

      <!-- Loading Animation -->
      <div class="loading-section">
        <div class="loading-spinner">
          <div class="spinner-ring"></div>
          <div class="spinner-ring"></div>
          <div class="spinner-ring"></div>
        </div>
      </div>

      <!-- Message Section -->
      <div class="message-section">
        <h2 class="main-message">Finalisation de l'authentification</h2>
        <p class="sub-message">
          Veuillez patienter pendant que nous finalisons votre connexion sécurisée.
        </p>
        <div class="loading-dots">
          <span class="dot"></span>
          <span class="dot"></span>
          <span class="dot"></span>
        </div>
      </div>

      <!-- Progress Indicator -->
      <div class="progress-section">
        <div class="progress-bar">
          <div class="progress-fill"></div>
        </div>
        <p class="progress-text">Authentification en cours...</p>
      </div>
    </div>
  </div>
</template>

<style scoped>
.callback-container {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: linear-gradient(135deg, #1e40af 0%, #7c3aed 100%);
  padding: 2rem;
  font-family: var(--font-family-sans, 'Inter', -apple-system, BlinkMacSystemFont, sans-serif);
}

.callback-content {
  max-width: 500px;
  width: 100%;
  text-align: center;
  background: rgba(255, 255, 255, 0.1);
  backdrop-filter: blur(20px);
  border: 1px solid rgba(255, 255, 255, 0.2);
  border-radius: 24px;
  padding: 3rem 2rem;
  box-shadow: 
    0 20px 40px rgba(0, 0, 0, 0.3),
    0 0 0 1px rgba(255, 255, 255, 0.1);
  animation: slideIn 0.6s ease-out;
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateY(20px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

/* Brand Section */
.brand-section {
  margin-bottom: 2.5rem;
}

.brand-icon {
  width: 4rem;
  height: 4rem;
  margin: 0 auto 1rem;
  color: white;
  display: flex;
  align-items: center;
  justify-content: center;
  background: rgba(255, 255, 255, 0.1);
  border-radius: 20px;
  animation: float 3s ease-in-out infinite;
}

@keyframes float {
  0%, 100% {
    transform: translateY(0px);
  }
  50% {
    transform: translateY(-10px);
  }
}

.brand-text {
  font-size: 2rem;
  font-weight: 700;
  color: white;
  margin: 0;
  letter-spacing: -0.025em;
  background: linear-gradient(45deg, #fff, #e0e7ff);
  -webkit-background-clip: text;
  -webkit-text-fill-color: transparent;
  background-clip: text;
}

/* Loading Section */
.loading-section {
  margin-bottom: 2.5rem;
}

.loading-spinner {
  position: relative;
  width: 80px;
  height: 80px;
  margin: 0 auto;
}

.spinner-ring {
  position: absolute;
  width: 100%;
  height: 100%;
  border: 3px solid transparent;
  border-top: 3px solid rgba(255, 255, 255, 0.8);
  border-radius: 50%;
  animation: spin 2s linear infinite;
}

.spinner-ring:nth-child(1) {
  animation-duration: 1.5s;
}

.spinner-ring:nth-child(2) {
  animation-duration: 2s;
  animation-direction: reverse;
  border-top-color: rgba(255, 255, 255, 0.6);
}

.spinner-ring:nth-child(3) {
  animation-duration: 2.5s;
  border-top-color: rgba(255, 255, 255, 0.4);
}

@keyframes spin {
  0% {
    transform: rotate(0deg);
  }
  100% {
    transform: rotate(360deg);
  }
}

/* Message Section */
.message-section {
  margin-bottom: 2.5rem;
}

.main-message {
  font-size: 1.5rem;
  font-weight: 600;
  color: white;
  margin: 0 0 0.75rem 0;
  letter-spacing: -0.025em;
}

.sub-message {
  font-size: 1rem;
  color: rgba(255, 255, 255, 0.8);
  margin: 0 0 1.5rem 0;
  line-height: 1.5;
}

.loading-dots {
  display: flex;
  justify-content: center;
  gap: 0.5rem;
}

.dot {
  width: 8px;
  height: 8px;
  background: rgba(255, 255, 255, 0.7);
  border-radius: 50%;
  animation: pulse 1.4s ease-in-out infinite;
}

.dot:nth-child(1) {
  animation-delay: 0s;
}

.dot:nth-child(2) {
  animation-delay: 0.2s;
}

.dot:nth-child(3) {
  animation-delay: 0.4s;
}

@keyframes pulse {
  0%, 80%, 100% {
    transform: scale(0.8);
    opacity: 0.5;
  }
  40% {
    transform: scale(1);
    opacity: 1;
  }
}

/* Progress Section */
.progress-section {
  margin-top: 2rem;
}

.progress-bar {
  width: 100%;
  height: 4px;
  background: rgba(255, 255, 255, 0.2);
  border-radius: 2px;
  overflow: hidden;
  margin-bottom: 1rem;
}

.progress-fill {
  height: 100%;
  background: linear-gradient(90deg, #60a5fa, #a78bfa, #60a5fa);
  background-size: 200% 100%;
  border-radius: 2px;
  animation: progress 2s ease-in-out infinite;
}

@keyframes progress {
  0% {
    width: 0%;
    background-position: 0% 50%;
  }
  50% {
    width: 70%;
    background-position: 100% 50%;
  }
  100% {
    width: 100%;
    background-position: 0% 50%;
  }
}

.progress-text {
  font-size: 0.875rem;
  color: rgba(255, 255, 255, 0.7);
  margin: 0;
}

/* Responsive Design */
@media (max-width: 768px) {
  .callback-container {
    padding: 1rem;
  }

  .callback-content {
    padding: 2rem 1.5rem;
    border-radius: 16px;
  }

  .brand-icon {
    width: 3rem;
    height: 3rem;
  }

  .brand-text {
    font-size: 1.5rem;
  }

  .main-message {
    font-size: 1.25rem;
  }

  .sub-message {
    font-size: 0.9rem;
  }

  .loading-spinner {
    width: 60px;
    height: 60px;
  }
}

@media (max-width: 480px) {
  .callback-content {
    padding: 1.5rem 1rem;
  }

  .brand-text {
    font-size: 1.25rem;
  }

  .main-message {
    font-size: 1.125rem;
  }
}

/* Dark mode compatibility */
@media (prefers-color-scheme: dark) {
  .callback-container {
    background: linear-gradient(135deg, #1e293b 0%, #581c87 100%);
  }
}

/* Accessibility */
@media (prefers-reduced-motion: reduce) {
  .callback-content,
  .brand-icon,
  .spinner-ring,
  .dot,
  .progress-fill {
    animation: none;
  }
}
</style>
