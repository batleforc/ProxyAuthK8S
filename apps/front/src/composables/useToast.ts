import { reactive } from 'vue';

/**
 * Short messages the layout shows in its live region (`Notices.vue`), in place
 * of maz-ui's toast plugin. Same call shape as maz-ui's `useToast`
 * (`success|info|warning|error(message, { timeout })`), so the stores keep
 * their logic and only change the import.
 */
export type NoticeKind = 'success' | 'info' | 'warning' | 'error';

export interface Notice {
  id: number;
  kind: NoticeKind;
  message: string;
}

export const notices = reactive<Notice[]>([]);

let nextId = 0;

export function dismissNotice(id: number) {
  const index = notices.findIndex((notice) => notice.id === id);
  if (index !== -1) notices.splice(index, 1);
}

function push(
  kind: NoticeKind,
  message: string,
  options?: { timeout?: number },
) {
  const id = ++nextId;
  notices.push({ id, kind, message });
  // Errors stay longer: they are what someone has to read.
  setTimeout(
    () => dismissNotice(id),
    options?.timeout ?? (kind === 'error' ? 8000 : 4000),
  );
}

export function useToast() {
  return {
    success: (message: string, options?: { timeout?: number }) =>
      push('success', message, options),
    info: (message: string, options?: { timeout?: number }) =>
      push('info', message, options),
    warning: (message: string, options?: { timeout?: number }) =>
      push('warning', message, options),
    error: (message: string, options?: { timeout?: number }) =>
      push('error', message, options),
  };
}
