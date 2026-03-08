import { generateFiles } from 'fumadocs-openapi';
import { openapi } from '@/lib/openapi';
import { rmSync, mkdirSync } from 'node:fs';

// Empty the output directory before generating new files.
const outputDir = './content/docs/api';
rmSync(outputDir, { recursive: true, force: true });
mkdirSync(outputDir, { recursive: true });

void generateFiles({
  input: openapi,
  output: outputDir,
  // we recommend to enable it
  // make sure your endpoint description doesn't break MDX syntax.
  includeDescription: true,
  groupBy: 'tag',
});
