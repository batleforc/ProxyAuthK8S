// Prints the CSP source for the inline theme script of the built index.html:
// 'sha256-<base64>'. Run at image build time; exits when the script is gone,
// since the CSP would then block nothing it should allow.
import { createHash } from 'node:crypto';
import { readFileSync } from 'node:fs';

const html = readFileSync(process.argv[2], 'utf8');
const inline = [...html.matchAll(/<script>([\s\S]*?)<\/script>/g)];
if (inline.length !== 1) {
  console.error(
    `expected one inline <script> in ${process.argv[2]}, found ${inline.length}`,
  );
  process.exit(1);
}
process.stdout.write(
  `'sha256-${createHash('sha256').update(inline[0][1]).digest('base64')}'`,
);
