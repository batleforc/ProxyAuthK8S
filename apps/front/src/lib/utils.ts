// Ported from batlehub/ui/src/lib/utils.ts (batlehub commit 3f18ab70, Apache-2.0).
import { type ClassValue, clsx } from 'clsx';
import { extendTailwindMerge } from 'tailwind-merge';

// `text-sub` and `text-display` are font sizes from @theme (assets/index.css):
// declared so a later one replaces `text-sm` instead of both being kept.
const twMerge = extendTailwindMerge({
  extend: { classGroups: { 'font-size': [{ text: ['sub', 'display'] }] } },
});

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}
