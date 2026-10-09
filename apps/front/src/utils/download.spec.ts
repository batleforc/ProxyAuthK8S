import { afterEach, describe, expect, it, vi } from 'vitest';
import { downloadTextFile } from './download';

describe('downloadTextFile', () => {
  const { createObjectURL: originalCreate, revokeObjectURL: originalRevoke } = URL;

  afterEach(() => {
    vi.restoreAllMocks();
    URL.createObjectURL = originalCreate;
    URL.revokeObjectURL = originalRevoke;
  });

  it('clicks a temporary link to a blob URL and cleans up', async () => {
    const createObjectURL = vi.fn(() => 'blob:fake');
    const revokeObjectURL = vi.fn();
    URL.createObjectURL = createObjectURL;
    URL.revokeObjectURL = revokeObjectURL;
    const click = vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(function (this: HTMLAnchorElement) {
      expect(this.download).toBe('kubeconfig.yaml');
      expect(this.href).toBe('blob:fake');
      expect(document.body.contains(this)).toBe(true);
    });

    downloadTextFile('apiVersion: v1', 'kubeconfig.yaml');

    expect(click).toHaveBeenCalledOnce();
    const blob = (createObjectURL.mock.calls[0] as unknown[])[0] as Blob;
    expect(blob.type).toBe('application/yaml');
    expect(await blob.text()).toBe('apiVersion: v1');
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:fake');
    expect(document.querySelector('a[download]')).toBeNull();
  });
});
