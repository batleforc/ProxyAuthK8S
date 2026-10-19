import { describe, expect, it, vi } from 'vitest';
import { installStaleChunkReload } from './staleChunk.ts';

function fakeWindow() {
  const target = new EventTarget();
  const store = new Map<string, string>();
  const reload = vi.fn();
  const win = {
    addEventListener: target.addEventListener.bind(target),
    sessionStorage: {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => store.set(k, v),
    },
    location: { reload },
  } as unknown as Window;
  const fire = () => {
    const event = new Event('vite:preloadError', { cancelable: true });
    target.dispatchEvent(event);
    return event.defaultPrevented;
  };
  return { win, reload, fire };
}

describe('installStaleChunkReload', () => {
  it('reloads once on a stale chunk, then lets the error through', () => {
    const { win, reload, fire } = fakeWindow();
    installStaleChunkReload(win);
    expect(fire()).toBe(true);
    expect(reload).toHaveBeenCalledTimes(1);
    // A second failure right after the reload is not a stale build.
    expect(fire()).toBe(false);
    expect(reload).toHaveBeenCalledTimes(1);
  });
});
