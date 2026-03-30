import fs from 'node:fs/promises';
import path from 'node:path';
import { extractProxyKubeApiSpecSchema } from '@/lib/proxykubeapi-crd';
import { ProxyKubeApiGeneratorClient } from '@/components/proxykubeapi-generator.client';

export async function ProxyKubeApiGenerator() {
  const crdPath = path.resolve(process.cwd(), '../deploy/crds.yaml');
  const crdYaml = await fs.readFile(crdPath, 'utf8');
  const specSchema = extractProxyKubeApiSpecSchema(crdYaml);

  return <ProxyKubeApiGeneratorClient specSchema={specSchema} />;
}
