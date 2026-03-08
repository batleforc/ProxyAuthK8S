import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

// fill this with your actual GitHub info, for example:
export const gitConfig = {
  user: 'batleforc',
  repo: 'proxyAuthK8s',
  branch: 'main',
  basePath: '.docs',
};

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: 'ProxyAuthK8s Docs',
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
