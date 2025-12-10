import { createRouter, createWebHistory } from 'vue-router';
import HomeView from '../views/HomeView.vue';

const router = createRouter({
  history: createWebHistory(import.meta.env.BASE_URL),
  routes: [
    {
      path: '/',
      name: 'home',
      component: HomeView,
      meta: {
        name: 'Home',
      },
    },
    {
      path: '/about',
      name: 'about',
      component: () => import('../views/AboutView.vue'),
      meta: { requiresAuth: true, name: 'About' },
    },
    {
      path: '/cli',
      name: 'cli',
      component: () => import('../views/CliView.vue'),
      meta: { requiresAuth: true, name: 'CLI' },
    },
    {
      path: '/auth/callback',
      children: [
        {
          path: '',
          name: 'callback',
          component: () => import('../views/CallbackView.vue'),
          meta: { name: 'Callback' },
        },
        {
          path: ':ns/:cluster',
          name: 'cluster-callback',
          component: () => import('../views/ClusterCallbackView.vue'),
          meta: { name: 'Cluster Callback' },
        },
      ],
    },
  ],
});

export default router;
