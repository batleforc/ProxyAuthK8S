import { useToast } from 'maz-ui/composables/useToast';

/**
 * Copy `text` to the clipboard and report the outcome with a toast.
 * `label` names what was copied in the (French) toast messages.
 */
export function useCopyToClipboard() {
  const toast = useToast();

  return async (text: string, label: string) => {
    try {
      await navigator.clipboard.writeText(text);
      toast.success(`${label} copié dans le presse-papiers !`);
    } catch (error) {
      toast.error(`Échec de la copie du ${label.toLowerCase()}.`);
      console.error('Clipboard copy failed:', error);
    }
  };
}
