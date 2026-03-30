'use client';

import { useDeferredValue, useMemo, useState } from 'react';
import { ChevronDown } from 'lucide-react';
import { stringify } from 'yaml';
import {
  getUnionOptionNames,
  type JsonSchema,
  sortedObjectProperties,
} from '@/lib/proxykubeapi-crd';

interface ProxyKubeApiGeneratorClientProps {
  specSchema: JsonSchema;
}

type FormObject = Record<string, unknown>;

interface PresetDefinition {
  id: string;
  label: string;
  description: string;
  apply: (specSchema: JsonSchema) => {
    metadataName: string;
    metadataNamespace: string;
    specValue: unknown;
  };
}

function isObject(value: unknown): value is FormObject {
  return typeof value === 'object' && value !== null && !Array.isArray(value);
}

function asObject(value: unknown): FormObject {
  return isObject(value) ? value : {};
}

function isObjectSchema(schema: JsonSchema): boolean {
  return schema.type === 'object' || Boolean(schema.properties) || Boolean(schema.oneOf?.length);
}

function isArraySchema(schema: JsonSchema): boolean {
  return schema.type === 'array' || Boolean(schema.items);
}

function scalarType(schema: JsonSchema): 'string' | 'number' | 'integer' | 'boolean' {
  if (schema.type === 'number' || schema.type === 'integer' || schema.type === 'boolean') {
    return schema.type;
  }

  return 'string';
}

function defaultScalarValue(schema: JsonSchema): unknown {
  if (schema.default !== undefined) {
    return schema.default;
  }

  if (schema.enum?.length) {
    return schema.enum[0];
  }

  switch (scalarType(schema)) {
    case 'boolean':
      return false;
    case 'number':
    case 'integer':
      return 0;
    default:
      return '';
  }
}

function createInitialValue(schema: JsonSchema): unknown {
  const unionOptions = getUnionOptionNames(schema);
  if (unionOptions.length > 0) {
    const option = unionOptions[0];
    const optionSchema = schema.properties?.[option] ?? { type: 'string' };

    return {
      [option]: createInitialValue(optionSchema),
    };
  }

  if (isObjectSchema(schema)) {
    const required = new Set(schema.required ?? []);
    const objectValue: FormObject = {};

    for (const [key, propertySchema] of sortedObjectProperties(schema)) {
      if (required.has(key)) {
        objectValue[key] = createInitialValue(propertySchema);
      }
    }

    return objectValue;
  }

  if (isArraySchema(schema)) {
    return [];
  }

  return defaultScalarValue(schema);
}

function pruneForManifest(value: unknown): unknown {
  if (value === undefined || value === null) {
    return undefined;
  }

  if (typeof value === 'string') {
    const trimmed = value.trim();
    return trimmed.length ? trimmed : undefined;
  }

  if (Array.isArray(value)) {
    return value
      .map((item) => pruneForManifest(item))
      .filter((item) => item !== undefined);
  }

  if (isObject(value)) {
    const next: FormObject = {};

    for (const [key, itemValue] of Object.entries(value)) {
      const pruned = pruneForManifest(itemValue);
      if (pruned !== undefined) {
        next[key] = pruned;
      }
    }

    return Object.keys(next).length > 0 ? next : undefined;
  }

  return value;
}

function validateRequired(schema: JsonSchema, value: unknown, path: string, required: boolean): string[] {
  if ((value === undefined || value === null) && required && !schema.nullable) {
    return [`${path} is required`];
  }

  return [];
}

function validateUnionField(schema: JsonSchema, value: unknown, path: string): string[] {
  const unionOptions = getUnionOptionNames(schema);

  if (!isObject(value)) {
    return [`${path} must be an object`];
  }

  const selectedOption = unionOptions.find((option) => value[option] !== undefined);
  if (!selectedOption) {
    return [`${path} must select one option`];
  }

  const optionSchema = schema.properties?.[selectedOption] ?? { type: 'string' };
  return validateField(optionSchema, value[selectedOption], `${path}.${selectedOption}`, true);
}

function validateObjectField(schema: JsonSchema, value: unknown, path: string): string[] {
  if (!isObject(value)) {
    return [`${path} must be an object`];
  }

  const requiredKeys = new Set(schema.required ?? []);

  return sortedObjectProperties(schema).flatMap(([key, propertySchema]) => {
    const child = value[key];
    const childRequired = requiredKeys.has(key);

    if (child === undefined && !childRequired) {
      return [];
    }

    return validateField(propertySchema, child, `${path}.${key}`, childRequired);
  });
}

function validateArrayField(schema: JsonSchema, value: unknown, path: string): string[] {
  if (!Array.isArray(value)) {
    return [`${path} must be an array`];
  }

  const itemSchema = schema.items ?? { type: 'string' };
  return value.flatMap((item, index) => validateField(itemSchema, item, `${path}[${index}]`, true));
}

function validateScalarType(expected: ReturnType<typeof scalarType>, value: unknown, path: string): string[] {
  const errors: string[] = [];

  if (expected === 'boolean' && typeof value !== 'boolean') {
    errors.push(`${path} must be a boolean`);
  }

  if ((expected === 'number' || expected === 'integer') && typeof value !== 'number') {
    errors.push(`${path} must be a number`);
  }

  if (expected === 'integer' && typeof value === 'number' && !Number.isInteger(value)) {
    errors.push(`${path} must be an integer`);
  }

  if (expected === 'string' && typeof value !== 'string') {
    errors.push(`${path} must be a string`);
  }

  return errors;
}

function validateScalarRange(schema: JsonSchema, value: unknown, path: string): string[] {
  const errors: string[] = [];

  if (typeof value === 'number' && schema.minimum !== undefined && value < schema.minimum) {
    errors.push(`${path} must be >= ${schema.minimum}`);
  }

  if (typeof value === 'number' && schema.maximum !== undefined && value > schema.maximum) {
    errors.push(`${path} must be <= ${schema.maximum}`);
  }

  return errors;
}

function validateScalarField(schema: JsonSchema, value: unknown, path: string, required: boolean): string[] {
  const errors: string[] = [];
  const expected = scalarType(schema);

  if (schema.enum?.length && !schema.enum.includes(value as never)) {
    errors.push(`${path} must be one of: ${schema.enum.join(', ')}`);
  }

  errors.push(...validateScalarType(expected, value, path));

  if (required && expected === 'string' && typeof value === 'string' && value.trim() === '') {
    errors.push(`${path} cannot be empty`);
  }

  errors.push(...validateScalarRange(schema, value, path));

  return errors;
}

function validateField(schema: JsonSchema, value: unknown, path: string, required: boolean): string[] {
  const requiredErrors = validateRequired(schema, value, path, required);
  if (requiredErrors.length > 0 || value === undefined || value === null) {
    return requiredErrors;
  }

  if (getUnionOptionNames(schema).length > 0) {
    return validateUnionField(schema, value, path);
  }

  if (isObjectSchema(schema)) {
    return validateObjectField(schema, value, path);
  }

  if (isArraySchema(schema)) {
    return validateArrayField(schema, value, path);
  }

  return validateScalarField(schema, value, path, required);
}

function labelFromKey(key: string): string {
  return key
    .replaceAll('_', ' ')
    .replaceAll(/([a-z0-9])([A-Z])/g, '$1 $2')
    .replace(/^./, (char) => char.toUpperCase());
}

function selectValue(value: unknown): string {
  if (typeof value === 'string' || typeof value === 'number' || typeof value === 'boolean') {
    return String(value);
  }

  return '';
}

function getPathDepth(path: string): number {
  return path
    .replaceAll(/\[\d+\]/g, '')
    .split('.')
    .filter(Boolean).length;
}

function shouldOpenSection(): boolean {
  return false;
}

function getSelectedUnionOption(value: unknown): string | undefined {
  const objectValue = asObject(value);
  return Object.keys(objectValue).find((key) => objectValue[key] !== undefined);
}

function buildPresetDefinitions(): PresetDefinition[] {
  return [
    {
      id: 'secret-backed',
      label: 'Secret-backed TLS',
      description: 'Cluster internal service using a CA bundle from a Secret.',
      apply: (specSchema) => {
        const specValue = createInitialValue(specSchema) as FormObject;

        specValue.cert = {
          Secret: {
            name: 'cluster-ca',
            key: 'ca.crt',
            namespace: 'infra',
          },
        };
        specValue.service = {
          KubernetesService: {
            name: 'kube-apiserver',
            namespace: 'kube-system',
            port: 443,
          },
        };
        specValue.enabled = true;
        specValue.expose_via_dashboard = true;

        return {
          metadataName: 'cluster-api-proxy',
          metadataNamespace: 'platform',
          specValue,
        };
      },
    },
    {
      id: 'external-oidc',
      label: 'External + OIDC',
      description: 'External API endpoint protected with an enabled OIDC provider.',
      apply: (specSchema) => {
        const specValue = createInitialValue(specSchema) as FormObject;

        specValue.cert = {
          Cert: 'LS0tLS1CRUdJTiBDRVJUSUZJQ0FURS0tLS0tCg==',
        };
        specValue.service = {
          ExternalService: {
            url: 'https://api.example.com',
          },
        };
        specValue.auth_config = {
          oidc_provider: {
            enabled: true,
            client_id: 'proxy-auth-k8s',
            issuer_url: 'https://sso.example.com/realms/platform',
            client_secret: '',
            extra_scope: 'profile email groups',
          },
          disable_validation: false,
          validate_against: 'OidcProvider',
          jwt: [],
        };
        specValue.enabled = true;
        specValue.dashboard_group = 'platform-admins';

        return {
          metadataName: 'external-platform-proxy',
          metadataNamespace: 'platform',
          specValue,
        };
      },
    },
    {
      id: 'configmap-guarded',
      label: 'ConfigMap + Guarded paths',
      description: 'Namespace-scoped access with a ConfigMap certificate source.',
      apply: (specSchema) => {
        const specValue = createInitialValue(specSchema) as FormObject;

        specValue.cert = {
          ConfigMap: {
            name: 'cluster-ca-config',
            key: 'ca.crt',
            namespace: 'infra',
          },
        };
        specValue.service = {
          KubernetesService: {
            name: 'kubernetes',
            namespace: 'default',
          },
        };
        specValue.security_config = {
          enabled: true,
          allowed_ressources: [
            {
              Path: {
                path: '/api/v1/namespaces/{{group}}/pods',
                parametised: true,
              },
            },
          ],
        };

        return {
          metadataName: 'team-pods-proxy',
          metadataNamespace: 'default',
          specValue,
        };
      },
    },
  ];
}

function OptionalRemoveButton({
  required,
  onRemove,
}: Readonly<{ required: boolean; onRemove: () => void }>) {
  if (required) {
    return null;
  }

  return (
    <button
      type="button"
      className="rounded-md border border-fd-border px-2 py-1 text-sm hover:bg-fd-accent"
      onClick={onRemove}
    >
      Remove field
    </button>
  );
}

interface FieldProps {
  name: string;
  path: string;
  schema: JsonSchema;
  value: unknown;
  required: boolean;
  onChange: (next: unknown) => void;
}

function OptionalFieldPlaceholder({
  name,
  schema,
  onChange,
}: Readonly<Pick<FieldProps, 'name' | 'schema' | 'onChange'>>) {
  return (
    <div className="rounded-lg border border-dashed border-fd-border p-3">
      <div className="text-sm text-fd-muted-foreground">{labelFromKey(name)} is optional.</div>
      <button
        type="button"
        className="mt-2 rounded-md border border-fd-border px-2 py-1 text-sm hover:bg-fd-accent"
        onClick={() => onChange(createInitialValue(schema))}
      >
        Add field
      </button>
    </div>
  );
}

function FieldDescription({ schema }: Readonly<{ schema: JsonSchema }>) {
  if (!schema.description) {
    return null;
  }

  return <p className="text-xs text-fd-muted-foreground">{schema.description}</p>;
}

function SectionShell({
  title,
  path,
  required,
  description,
  meta,
  children,
}: Readonly<{
  title: string;
  path: string;
  required: boolean;
  description?: string;
  meta?: string;
  children: React.ReactNode;
}>) {
  return (
    <details className="rounded-xl border border-fd-border bg-fd-card/30" open={shouldOpenSection()}>
      <summary className="flex cursor-pointer list-none items-start justify-between gap-3 p-3 [&::-webkit-details-marker]:hidden">
        <div className="space-y-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold">{title}</span>
            <span className="rounded-full border border-fd-border px-2 py-0.5 text-[11px] uppercase tracking-wide text-fd-muted-foreground">
              {required ? 'Required' : 'Optional'}
            </span>
            {meta ? (
              <span className="rounded-full bg-fd-secondary px-2 py-0.5 text-[11px] text-fd-muted-foreground">
                {meta}
              </span>
            ) : null}
          </div>
          {description ? <p className="text-xs text-fd-muted-foreground">{description}</p> : null}
        </div>
        <span className="inline-flex items-center text-fd-muted-foreground" aria-hidden>
          <ChevronDown className="size-4" />
        </span>
      </summary>
      <div className="border-t border-fd-border p-3">{children}</div>
    </details>
  );
}

function PresetCard({
  preset,
  active,
  onSelect,
}: Readonly<{
  preset: PresetDefinition;
  active: boolean;
  onSelect: () => void;
}>) {
  return (
    <button
      type="button"
      className={[
        'rounded-xl border p-3 text-left transition-colors',
        active
          ? 'border-fd-primary bg-fd-primary/15 shadow-sm ring-1 ring-fd-primary/40'
          : 'border-fd-border bg-fd-card hover:bg-fd-accent',
      ].join(' ')}
      onClick={onSelect}
    >
      <div className="flex items-center justify-between gap-2">
        <div className="text-sm font-semibold text-fd-foreground">{preset.label}</div>
        {active ? (
          <span className="rounded-full bg-fd-primary px-2 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-fd-primary-foreground">
            Active
          </span>
        ) : null}
      </div>
      <p className={['mt-1 text-xs', active ? 'text-fd-foreground/90' : 'text-fd-muted-foreground'].join(' ')}>
        {preset.description}
      </p>
    </button>
  );
}

function UnionField({ name, path, schema, value, required, onChange }: Readonly<FieldProps>) {
  const objectValue = asObject(value);
  const unionOptions = getUnionOptionNames(schema);
  const selected = unionOptions.find((option) => objectValue[option] !== undefined) ?? unionOptions[0];
  const selectedSchema = schema.properties?.[selected] ?? { type: 'string' };
  const isInsecureCertVariant = name === 'cert' && selected === 'Insecure';

  return (
    <SectionShell
      title={labelFromKey(name)}
      path={path}
      required={required}
      description={schema.description}
      meta={`Variant: ${selected}`}
    >
      <div className="space-y-3">
        {isInsecureCertVariant ? (
          <div className="rounded-lg border border-amber-300 bg-amber-50 px-3 py-2 text-xs text-amber-900 dark:border-amber-700 dark:bg-amber-950/30 dark:text-amber-200">
            Warning: la variante Insecure est presente pour la compatibilite du schema CRD mais elle est volontairement non fonctionnelle.
          </div>
        ) : null}
        <label className="flex flex-col gap-1 text-sm">
          <span className="font-medium">Variant</span>
          <select
            className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
            value={selected}
            onChange={(event) => {
              const nextOption = event.target.value;
              const optionSchema = schema.properties?.[nextOption] ?? { type: 'string' };
              onChange({ [nextOption]: createInitialValue(optionSchema) });
            }}
          >
            {unionOptions.map((option) => (
              <option key={option} value={option}>
                {option}
              </option>
            ))}
          </select>
        </label>
        <FieldRenderer
          name={selected}
          path={`${path}.${selected}`}
          schema={selectedSchema}
          value={objectValue[selected]}
          required
          onChange={(nextValue) => onChange({ [selected]: nextValue })}
        />
        <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
      </div>
    </SectionShell>
  );
}

function ObjectField({ name, path, schema, value, required, onChange }: Readonly<FieldProps>) {
  const objectValue = asObject(value);
  const requiredKeys = new Set(schema.required ?? []);
  const propertyCount = sortedObjectProperties(schema).length;

  return (
    <SectionShell
      title={labelFromKey(name)}
      path={path}
      required={required}
      description={schema.description}
      meta={`${propertyCount} field${propertyCount === 1 ? '' : 's'}`}
    >
      <div className="space-y-3">
        {sortedObjectProperties(schema).map(([childName, childSchema]) => (
          <FieldRenderer
            key={`${path}.${childName}`}
            name={childName}
            path={`${path}.${childName}`}
            schema={childSchema}
            value={objectValue[childName]}
            required={requiredKeys.has(childName)}
            onChange={(nextValue) => {
              const nextObject = { ...objectValue };

              if (nextValue === undefined) {
                delete nextObject[childName];
              } else {
                nextObject[childName] = nextValue;
              }

              onChange(nextObject);
            }}
          />
        ))}
        <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
      </div>
    </SectionShell>
  );
}

function ArrayField({ name, path, schema, value, required, onChange }: Readonly<FieldProps>) {
  const items = Array.isArray(value) ? value : [];
  const itemSchema = schema.items ?? { type: 'string' };

  return (
    <SectionShell
      title={labelFromKey(name)}
      path={path}
      required={required}
      description={schema.description}
      meta={`${items.length} item${items.length === 1 ? '' : 's'}`}
    >
      <div className="space-y-3">
        {items.length === 0 ? <p className="text-sm text-fd-muted-foreground">No item yet.</p> : null}
        {items.map((item, index) => (
          <div key={`${path}[${index}]`} className="space-y-2 rounded-md border border-fd-border bg-fd-background/60 p-3">
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-fd-muted-foreground">Item {index + 1}</span>
              <button
                type="button"
                className="rounded-md border border-fd-border px-2 py-0.5 text-xs hover:bg-fd-accent"
                onClick={() => {
                  const nextItems = [...items];
                  nextItems.splice(index, 1);
                  onChange(nextItems);
                }}
              >
                Remove
              </button>
            </div>
            <FieldRenderer
              name={`${name}[${index}]`}
              path={`${path}[${index}]`}
              schema={itemSchema}
              value={item}
              required
              onChange={(nextValue) => {
                const nextItems = [...items];
                nextItems[index] = nextValue;
                onChange(nextItems);
              }}
            />
          </div>
        ))}
        <div className="flex gap-2">
          <button
            type="button"
            className="rounded-md border border-fd-border px-2 py-1 text-sm hover:bg-fd-accent"
            onClick={() => onChange([...items, createInitialValue(itemSchema)])}
          >
            Add item
          </button>
          <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
        </div>
      </div>
    </SectionShell>
  );
}

function ScalarField({ name, schema, value, required, onChange }: Readonly<Omit<FieldProps, 'path'>>) {
  const label = (
    <span className="font-medium">
      {labelFromKey(name)}
      {required ? ' *' : ''}
    </span>
  );

  if (scalarType(schema) === 'boolean') {
    return (
      <label className="flex items-center gap-2 rounded-lg border border-fd-border p-3 text-sm">
        <input
          type="checkbox"
          checked={Boolean(value)}
          onChange={(event) => onChange(event.target.checked)}
        />
        <span>
          {labelFromKey(name)}
          {required ? ' *' : ''}
        </span>
      </label>
    );
  }

  if (schema.enum?.length) {
    return (
      <label className="flex flex-col gap-1 text-sm">
        {label}
        <FieldDescription schema={schema} />
        <select
          className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
          value={selectValue(value)}
          onChange={(event) => {
            const selected = event.target.value;
            const enumValue = schema.enum?.find((entry) => String(entry) === selected) ?? selected;
            onChange(enumValue);
          }}
        >
          {schema.enum.map((entry) => (
            <option key={String(entry)} value={String(entry)}>
              {String(entry)}
            </option>
          ))}
        </select>
        <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
      </label>
    );
  }

  if (scalarType(schema) === 'number' || scalarType(schema) === 'integer') {
    const type = scalarType(schema);

    return (
      <label className="flex flex-col gap-1 text-sm">
        {label}
        <FieldDescription schema={schema} />
        <input
          type="number"
          step={type === 'integer' ? 1 : 'any'}
          min={schema.minimum}
          max={schema.maximum}
          className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
          value={typeof value === 'number' ? value : 0}
          onChange={(event) => {
            const next = event.target.value;
            onChange(next === '' ? undefined : Number(next));
          }}
        />
        <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
      </label>
    );
  }

  return (
    <label className="flex flex-col gap-1 text-sm">
      {label}
      <FieldDescription schema={schema} />
      <input
        type={schema.format === 'uri' ? 'url' : 'text'}
        className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
        value={typeof value === 'string' ? value : ''}
        onChange={(event) => onChange(event.target.value)}
      />
      <OptionalRemoveButton required={required} onRemove={() => onChange(undefined)} />
    </label>
  );
}

function FieldRenderer({ name, path, schema, value, required, onChange }: Readonly<FieldProps>) {
  if (!required && value === undefined) {
    return <OptionalFieldPlaceholder name={name} schema={schema} onChange={onChange} />;
  }

  if (getUnionOptionNames(schema).length > 0) {
    return <UnionField name={name} path={path} schema={schema} value={value} required={required} onChange={onChange} />;
  }

  if (isArraySchema(schema)) {
    return <ArrayField name={name} path={path} schema={schema} value={value} required={required} onChange={onChange} />;
  }

  if (isObjectSchema(schema)) {
    return <ObjectField name={name} path={path} schema={schema} value={value} required={required} onChange={onChange} />;
  }

  return <ScalarField name={name} schema={schema} value={value} required={required} onChange={onChange} />;
}

export function ProxyKubeApiGeneratorClient({ specSchema }: Readonly<ProxyKubeApiGeneratorClientProps>) {
  const presets = useMemo(() => buildPresetDefinitions(), []);
  const [metadataName, setMetadataName] = useState('');
  const [metadataNamespace, setMetadataNamespace] = useState('default');
  const [specValue, setSpecValue] = useState<unknown>(() => createInitialValue(specSchema));
  const [copied, setCopied] = useState(false);
  const [activePresetId, setActivePresetId] = useState<string | null>(null);

  const errors = useMemo(() => {
    const baseErrors: string[] = [];

    if (metadataName.trim() === '') {
      baseErrors.push('metadata.name is required');
    }

    return baseErrors.concat(validateField(specSchema, specValue, 'spec', true));
  }, [metadataName, specSchema, specValue]);
  const deferredErrors = useDeferredValue(errors);

  const manifestYaml = useMemo(() => {
    const prunedSpec = pruneForManifest(specValue);

    const manifest = {
      apiVersion: 'weebo.si.rs/v1',
      kind: 'ProxyKubeApi',
      metadata: {
        name: metadataName.trim(),
        ...(metadataNamespace.trim() ? { namespace: metadataNamespace.trim() } : {}),
      },
      ...(prunedSpec ? { spec: prunedSpec } : {}),
    };

    return stringify(manifest, {
      indent: 2,
      lineWidth: 100,
      minContentWidth: 0,
    });
  }, [metadataName, metadataNamespace, specValue]);
  const deferredManifestYaml = useDeferredValue(manifestYaml);
  const specObject = asObject(specValue);
  const selectedCertVariant = getSelectedUnionOption(specObject.cert) ?? 'Not set';
  const selectedServiceVariant = getSelectedUnionOption(specObject.service) ?? 'Not set';

  const applyPreset = (preset: PresetDefinition) => {
    const nextState = preset.apply(specSchema);

    setMetadataName(nextState.metadataName);
    setMetadataNamespace(nextState.metadataNamespace);
    setSpecValue(nextState.specValue);
    setActivePresetId(preset.id);
  };

  const downloadYaml = () => {
    const blob = new Blob([deferredManifestYaml], { type: 'text/yaml;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const anchor = document.createElement('a');
    const safeName = metadataName.trim() || 'proxykubeapi';

    anchor.href = url;
    anchor.download = `${safeName}.yaml`;
    anchor.click();
    URL.revokeObjectURL(url);
  };

  return (
    <div className="space-y-6">
      <div className="rounded-2xl border border-fd-border bg-linear-to-br from-fd-card via-fd-card to-fd-secondary/50 p-5">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div>
            <h2 className="text-lg font-semibold">ProxyKubeApi Generator</h2>
            <p className="mt-1 max-w-2xl text-sm text-fd-muted-foreground">
              Build a manifest from the live CRD schema, switch variants as needed, and export YAML when the validation panel is clean.
            </p>
          </div>
          <div className="grid min-w-60 gap-2 sm:grid-cols-3">
            <div className="rounded-xl border border-fd-border bg-fd-background/70 p-3">
              <div className="text-[11px] uppercase tracking-wide text-fd-muted-foreground">Errors</div>
              <div className="mt-1 text-lg font-semibold">{deferredErrors.length}</div>
            </div>
            <div className="rounded-xl border border-fd-border bg-fd-background/70 p-3">
              <div className="text-[11px] uppercase tracking-wide text-fd-muted-foreground">Cert</div>
              <div className="mt-1 text-sm font-semibold">{selectedCertVariant}</div>
            </div>
            <div className="rounded-xl border border-fd-border bg-fd-background/70 p-3">
              <div className="text-[11px] uppercase tracking-wide text-fd-muted-foreground">Service</div>
              <div className="mt-1 text-sm font-semibold">{selectedServiceVariant}</div>
            </div>
          </div>
        </div>
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <div className="space-y-4">
          <div className="rounded-xl border border-fd-border bg-fd-card p-4">
            <div className="flex items-center justify-between gap-3">
              <div>
                <h3 className="text-sm font-semibold">Preset examples</h3>
                <p className="mt-1 text-xs text-fd-muted-foreground">
                  Start from a common setup, then fine-tune the schema-driven fields below.
                </p>
              </div>
            </div>
            <div className="mt-3 grid gap-3 sm:grid-cols-2">
              {presets.map((preset) => (
                <PresetCard
                  key={preset.id}
                  preset={preset}
                  active={preset.id === activePresetId}
                  onSelect={() => applyPreset(preset)}
                />
              ))}
            </div>
          </div>

          <div className="rounded-xl border border-fd-border bg-fd-card/30">
            <div className="flex items-start justify-between gap-3 p-3">
              <div className="space-y-1">
                <div className="flex flex-wrap items-center gap-2">
                  <span className="text-sm font-semibold">Metadata</span>
                  <span className="rounded-full border border-fd-border px-2 py-0.5 text-[11px] uppercase tracking-wide text-fd-muted-foreground">
                    Required
                  </span>
                  <span className="rounded-full bg-fd-secondary px-2 py-0.5 text-[11px] text-fd-muted-foreground">
                    2 fields
                  </span>
                </div>
                <p className="text-xs text-fd-muted-foreground">
                  Resource identity fields are kept outside the CRD-driven spec so you can reuse presets across namespaces.
                </p>
              </div>
            </div>
            <div className="space-y-3 border-t border-fd-border p-3">
              <label className="flex flex-col gap-1 text-sm">
                <span className="font-medium">Name *</span>
                <input
                  className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
                  value={metadataName}
                  onChange={(event) => {
                    setMetadataName(event.target.value);
                    setActivePresetId(null);
                  }}
                  placeholder="my-proxykubeapi"
                />
              </label>
              <label className="flex flex-col gap-1 text-sm">
                <span className="font-medium">Namespace</span>
                <input
                  className="rounded-md border border-fd-border bg-fd-background px-2 py-1"
                  value={metadataNamespace}
                  onChange={(event) => {
                    setMetadataNamespace(event.target.value);
                    setActivePresetId(null);
                  }}
                  placeholder="default"
                />
              </label>
            </div>
          </div>

          <FieldRenderer
            name="spec"
            path="spec"
            schema={specSchema}
            value={specValue}
            required
            onChange={setSpecValue}
          />

          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              className="rounded-md border border-fd-border px-3 py-1.5 text-sm hover:bg-fd-accent"
              onClick={() => {
                setMetadataName('');
                setMetadataNamespace('default');
                setSpecValue(createInitialValue(specSchema));
                setActivePresetId(null);
              }}
            >
              Reset
            </button>
            <div className="self-center text-xs text-fd-muted-foreground">
              Tip: sections are collapsed by default. Expand only what you need.
            </div>
          </div>
        </div>

        <div className="space-y-4 lg:sticky lg:top-6 lg:self-start">
          <div className="rounded-xl border border-fd-border bg-fd-card p-4">
            <h3 className="text-sm font-semibold">Validation</h3>
            {deferredErrors.length > 0 ? (
              <ul className="mt-2 list-disc space-y-1 pl-5 text-sm text-red-600">
                {deferredErrors.map((error) => (
                  <li key={error}>{error}</li>
                ))}
              </ul>
            ) : (
              <p className="mt-2 text-sm text-green-700">Manifest is valid against required schema constraints.</p>
            )}
          </div>

          <div className="rounded-xl border border-fd-border bg-fd-card p-4">
            <div className="flex flex-wrap items-center justify-between gap-2">
              <h3 className="text-sm font-semibold">Generated YAML</h3>
              <div className="flex gap-2">
                <button
                  type="button"
                  className="rounded-md border border-fd-border px-2 py-1 text-xs hover:bg-fd-accent"
                  onClick={async () => {
                    await navigator.clipboard.writeText(deferredManifestYaml);
                    setCopied(true);
                    globalThis.setTimeout(() => setCopied(false), 1200);
                  }}
                >
                  {copied ? 'Copied' : 'Copy'}
                </button>
                <button
                  type="button"
                  className="rounded-md border border-fd-border px-2 py-1 text-xs hover:bg-fd-accent"
                  onClick={downloadYaml}
                >
                  Download
                </button>
              </div>
            </div>
            <p className="mt-1 text-xs text-fd-muted-foreground">
              Preview updates live as you edit the form. Empty optional values are omitted from the manifest.
            </p>
            <pre className="mt-3 max-h-[70vh] overflow-auto rounded-xl bg-fd-secondary/50 p-3 text-xs">
              <code>{deferredManifestYaml}</code>
            </pre>
          </div>
        </div>
      </div>
    </div>
  );
}
