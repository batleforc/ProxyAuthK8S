'use client';
import { cn } from '../../lib/cn';
import { buttonVariants } from '../ui/button';
import { MessageSquare } from 'lucide-react';

/**
 * A feedback component to be attached at the end of page - redirects to GitHub discussions
 */
export function Feedback() {
  return (
    <div className="border-y py-3 flex flex-row items-center gap-2">
      <p className="text-sm font-medium pe-2">Have feedback?</p>
      <a
        href="https://github.com/batleforc/proxyAuthK8s/discussions"
        rel="noreferrer noopener"
        target="_blank"
        className={cn(
          buttonVariants({
            color: 'primary',
          }),
          'text-xs gap-1.5',
        )}
      >
        <MessageSquare className="size-3.5" />
        Share on GitHub
      </a>
    </div>
  );
}
