import { describe, expect, it } from 'vitest';
import { safeRedirectTarget } from './redirect';

describe('safeRedirectTarget', () => {
  it('accepts absolute http(s) URLs', () => {
    expect(safeRedirectTarget('https://idp.example.com/authorize?state=x')).toBe(
      'https://idp.example.com/authorize?state=x',
    );
    expect(safeRedirectTarget('http://localhost:18000/callback')).toBe('http://localhost:18000/callback');
  });

  it.each([
    'javascript:alert(1)',
    'JaVaScRiPt:alert(1)',
    'data:text/html,<script>alert(1)</script>',
    'file:///etc/passwd',
    'ftp://example.com',
  ])('refuses the non-http scheme %s', (url) => {
    expect(safeRedirectTarget(url)).toBeNull();
  });

  it.each(['', '   ', '/relative/path', 'not a url', undefined, null, 42, {}])(
    'refuses non-absolute or non-string input %s',
    (value) => {
      expect(safeRedirectTarget(value)).toBeNull();
    },
  );
});
