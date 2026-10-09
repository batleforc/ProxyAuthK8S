import { beforeEach, describe, expect, it, vi } from 'vitest';

const toast = vi.hoisted(() => ({ success: vi.fn(), error: vi.fn() }));
vi.mock('maz-ui/composables/useToast', () => ({ useToast: () => toast }));

import { useCopyToClipboard } from './useCopyToClipboard';

describe('useCopyToClipboard', () => {
  const writeText = vi.fn();

  beforeEach(() => {
    vi.clearAllMocks();
    Object.defineProperty(navigator, 'clipboard', { value: { writeText }, configurable: true });
  });

  it('writes the text and confirms with the label', async () => {
    writeText.mockResolvedValue(undefined);
    await useCopyToClipboard()('secret', 'Access Token');

    expect(writeText).toHaveBeenCalledWith('secret');
    expect(toast.success).toHaveBeenCalledWith('Access Token copié dans le presse-papiers !');
    expect(toast.error).not.toHaveBeenCalled();
  });

  it('reports a failure with the lower-cased label', async () => {
    writeText.mockRejectedValue(new Error('denied'));
    vi.spyOn(console, 'error').mockImplementation(() => undefined);
    await useCopyToClipboard()('secret', 'Kubeconfig');

    expect(toast.error).toHaveBeenCalledWith('Échec de la copie du kubeconfig.');
    expect(toast.success).not.toHaveBeenCalled();
  });
});
