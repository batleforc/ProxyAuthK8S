/**
 * After a redeploy, a tab still running the previous build asks for hashed
 * chunks that no longer exist (nginx answers index.html), so a lazy route or
 * an async component would render nothing. Vite reports it as
 * `vite:preloadError`: reload once to pick up the new build. The marker stops
 * a reload loop when the chunk is missing for another reason.
 */
const RELOADED_AT = 'proxyauthk8s:stale-chunk-reload';
const RETRY_AFTER_MS = 10_000;

export function installStaleChunkReload(win: Window = window): void {
  win.addEventListener('vite:preloadError', (event) => {
    let last = 0;
    try {
      last = Number(win.sessionStorage.getItem(RELOADED_AT) ?? 0);
      win.sessionStorage.setItem(RELOADED_AT, String(Date.now()));
    } catch {
      // Storage blocked: still reload, at worst once per error.
    }
    if (Date.now() - last < RETRY_AFTER_MS) return;
    event.preventDefault();
    win.location.reload();
  });
}
