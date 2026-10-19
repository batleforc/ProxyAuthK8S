import { i18n } from '../i18n';
import { useToast } from './useToast';

/**
 * Copy `text` to the clipboard and announce the outcome. `label` names what
 * was copied in the message.
 */
export function useCopyToClipboard() {
  const toast = useToast();
  const { t } = i18n.global;

  return async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(t('copy.done', { label }));
    } catch (error) {
      toast.error(t('copy.failed', { label: label.toLowerCase() }));
      console.error('Clipboard copy failed:', error);
    }
  };
}
