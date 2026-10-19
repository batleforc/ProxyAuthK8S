import vue from 'eslint-plugin-vue';
import tseslint from 'typescript-eslint';
import oxlint from 'eslint-plugin-oxlint';
import skipFormatting from '@vue/eslint-config-prettier/skip-formatting';
import baseConfig from '../../eslint.config.mjs';

export default [
  ...baseConfig,
  ...vue.configs['flat/recommended'],
  {
    files: ['**/*.vue'],
    languageOptions: {
      parserOptions: {
        parser: tseslint.parser,
      },
    },
  },
  {
    files: ['**/*.ts', '**/*.tsx', '**/*.js', '**/*.jsx', '**/*.vue'],
    rules: {
      'vue/multi-word-component-names': 'off',
      // T2: HTML rendering of server data is refused everywhere (code-block
      // renders text too).
      'vue/no-v-html': 'error',
    },
  },
  {
    // Node scripts (i18n check, CSP hash), not browser code.
    files: ['scripts/**/*.mjs'],
    languageOptions: { globals: { process: 'readonly', console: 'readonly' } },
  },
  {
    // Ported from batlehub as is (task ui:diff-batlehub): optional props
    // without defaults are how the source writes them.
    files: ['src/components/ui/**/*.vue'],
    rules: { 'vue/require-default-prop': 'off' },
  },
  {
    // The suite on the real stack (RFC 0009 §6.4) intercepts nothing: a
    // mocked route there would prove the mock, not the product.
    files: ['e2e-it/**/*.ts'],
    rules: {
      'no-restricted-properties': [
        'error',
        {
          object: 'page',
          property: 'route',
          message: 'e2e-it runs on the real stack: no page.route.',
        },
        {
          object: 'context',
          property: 'route',
          message: 'e2e-it runs on the real stack: no route.',
        },
      ],
    },
  },
  // Prettier owns the format (create-vue's shape).
  skipFormatting,
  ...oxlint.buildFromOxlintConfigFile(
    `${import.meta.dirname}/../../.oxlintrc.json`,
  ),
];
