import type { IconComponent } from '@maz-ui/icons';

export interface NavItem {
  label: string;
  to: string;
  icon: IconComponent;
}

// A type alias (not an interface) so it stays assignable to maz-ui's
// `MazDropdownMenuItem`, which is a `Record<string, unknown>`.
export type UserMenuItem = {
  label: string;
  onClick: () => unknown;
  class: string;
};
