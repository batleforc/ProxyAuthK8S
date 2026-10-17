import { describe, expect, it } from 'vitest';
import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join, relative } from 'node:path';

/**
 * RFC 0002 §4.3 / §5.2: no colour is written anywhere but tokens.css, so a
 * view cannot paint a dark and a light rendition at once.
 */
const SRC = join(import.meta.dirname, '..');
const ALLOWED = new Set(['design/tokens.css']);
const LITERAL = /#[0-9a-f]{3,8}\b|\b(?:rgba?|hsla?|oklch|oklab|lab|lch|hwb)\(/i;

function files(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) return files(path);
    return /\.(vue|css|ts)$/.test(name) && !name.endsWith('.spec.ts')
      ? [path]
      : [];
  });
}

describe('no literal colour outside tokens.css', () => {
  it('finds none', () => {
    const offenders = files(SRC)
      .filter((path) => !ALLOWED.has(relative(SRC, path)))
      .flatMap((path) =>
        readFileSync(path, 'utf8')
          .split('\n')
          .map((line, index) => ({ line, index }))
          .filter(({ line }) => LITERAL.test(line))
          .map(
            ({ line, index }) =>
              `${relative(SRC, path)}:${index + 1}: ${line.trim()}`,
          ),
      );
    expect(offenders).toEqual([]);
  });
});
