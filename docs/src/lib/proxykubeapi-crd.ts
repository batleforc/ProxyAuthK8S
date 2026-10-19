import { parseAllDocuments } from 'yaml';

export type JsonPrimitive = string | number | boolean | null;

export interface JsonSchema {
  type?: string;
  description?: string;
  title?: string;
  nullable?: boolean;
  enum?: JsonPrimitive[];
  default?: JsonPrimitive | JsonPrimitive[] | Record<string, unknown>;
  required?: string[];
  properties?: Record<string, JsonSchema>;
  oneOf?: JsonSchema[];
  items?: JsonSchema;
  format?: string;
  minimum?: number;
  maximum?: number;
}

interface CrdVersionSchema {
  schema?: {
    openAPIV3Schema?: JsonSchema;
  };
  storage?: boolean;
}

interface CrdDocument {
  kind?: string;
  metadata?: {
    name?: string;
  };
  spec?: {
    names?: {
      kind?: string;
    };
    versions?: CrdVersionSchema[];
  };
}

export function extractProxyKubeApiSpecSchema(crdYaml: string): JsonSchema {
  const docs = parseAllDocuments(crdYaml)
    .map((doc) => doc.toJSON() as CrdDocument)
    .filter(Boolean);

  const proxyKubeApiCrd = docs.find(
    (doc) =>
      doc.kind === 'CustomResourceDefinition' &&
      (doc.metadata?.name === 'proxykubeapis.weebo.si.rs' ||
        doc.spec?.names?.kind === 'ProxyKubeApi'),
  );

  if (!proxyKubeApiCrd?.spec?.versions?.length) {
    throw new Error(
      'Unable to find ProxyKubeApi CRD versions in deploy/crds.yaml',
    );
  }

  const selectedVersion =
    proxyKubeApiCrd.spec.versions.find((version) => version.storage) ??
    proxyKubeApiCrd.spec.versions[0];

  const rootSchema = selectedVersion?.schema?.openAPIV3Schema;
  const specSchema = rootSchema?.properties?.spec;

  if (!rootSchema || !specSchema) {
    throw new Error('Unable to resolve ProxyKubeApi spec schema from CRD');
  }

  return specSchema;
}

export function sortedObjectProperties(
  schema: JsonSchema,
): Array<[string, JsonSchema]> {
  const entries = Object.entries(schema.properties ?? {});
  const required = new Set(schema.required ?? []);

  return entries.sort(([left], [right]) => {
    const leftRequired = required.has(left) ? 0 : 1;
    const rightRequired = required.has(right) ? 0 : 1;

    if (leftRequired !== rightRequired) {
      return leftRequired - rightRequired;
    }

    return left.localeCompare(right);
  });
}

export function getUnionOptionNames(schema: JsonSchema): string[] {
  if (!schema.oneOf?.length) {
    return [];
  }

  const options = new Set<string>();

  for (const branch of schema.oneOf) {
    for (const key of branch.required ?? []) {
      if (schema.properties?.[key]) {
        options.add(key);
      }
    }
  }

  return [...options];
}
