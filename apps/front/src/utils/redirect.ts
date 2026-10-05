/**
 * Validate a redirect target received from the API before navigating to it.
 *
 * Only absolute http(s) URLs are accepted, so a malformed or
 * attacker-influenced response cannot turn a redirect into an open redirect to
 * another scheme or a `javascript:` / `data:` sink.
 *
 * @returns the normalised URL to navigate to, or `null` when it must be refused.
 */
export function safeRedirectTarget(raw: unknown): string | null {
  if (typeof raw !== 'string' || raw.trim() === '') {
    return null;
  }
  try {
    const target = new URL(raw);
    if (target.protocol !== 'https:' && target.protocol !== 'http:') {
      return null;
    }
    return target.href;
  } catch {
    return null;
  }
}
