import { readFileSync } from 'node:fs';
import { visit } from 'unist-util-visit';

const rootPkg = JSON.parse(readFileSync('../package.json', 'utf-8'));

const variables = {
  __APP_VERSION__: rootPkg.version,
};

/** Remark plugin that replaces placeholders like __APP_VERSION__ in all text and code nodes. */
export function remarkVariables() {
  return (tree) => {
    visit(tree, (node) => {
      if (node.type === 'text' || node.type === 'inlineCode' || node.type === 'code') {
        for (const [placeholder, value] of Object.entries(variables)) {
          if (node.value?.includes(placeholder)) {
            node.value = node.value.replaceAll(placeholder, value);
          }
        }
      }
    });
  };
}
