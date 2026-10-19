import { generateFiles } from 'fumadocs-openapi';
import { openapi } from '@/lib/openapi';
import { readdirSync, rmSync, mkdirSync } from 'node:fs';
import { join } from 'node:path';

// Empty the output directory before generating new files, keeping the
// hand-written landing page (`index.mdx`).
const outputDir = './content/docs/api';
mkdirSync(outputDir, { recursive: true });
for (const entry of readdirSync(outputDir)) {
  if (entry === 'index.mdx') continue;
  rmSync(join(outputDir, entry), { recursive: true, force: true });
}

void generateFiles({
  input: openapi,
  output: outputDir,
  // we recommend to enable it
  // make sure your endpoint description doesn't break MDX syntax.
  includeDescription: true,
  groupBy: 'tag',
});
