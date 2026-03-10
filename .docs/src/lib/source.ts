import { docs } from 'fumadocs-mdx:collections/server';
import { type InferPageType, loader } from 'fumadocs-core/source';
import { openapiPlugin } from 'fumadocs-openapi/server';
import { createElement } from 'react';
import { icons } from 'lucide-react';
import * as simpleIcons from '@icons-pack/react-simple-icons';

// See https://fumadocs.dev/docs/headless/source-api for more info
export const source = loader({
  baseUrl: '/docs',
  source: docs.toFumadocsSource(),
  plugins: [openapiPlugin()],
  icon(icon) {
    if (!icon) {
      // You may set a default icon
      return;
    }
    if (icon in icons) return createElement(icons[icon as keyof typeof icons]);

    const customIcon = simpleIcons[icon as keyof typeof simpleIcons];
    if (typeof customIcon === 'function') {
      return createElement(customIcon);
    }
  },
});

export function getPageImage(page: InferPageType<typeof source>) {
  const segments = [...page.slugs, 'image.webp'];

  return {
    segments,
    url: `/og/docs/${segments.join('/')}`,
  };
}

export async function getLLMText(page: InferPageType<typeof source>) {
  const processed = await page.data.getText('processed');

  return `# ${page.data.title}

${processed}`;
}
