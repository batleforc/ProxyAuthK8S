import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { baseOptions } from '@weebo/operator-docs/site';
import { site } from '@/lib/shared';

export default function Layout({ children }: LayoutProps<'/'>) {
  return <HomeLayout {...baseOptions(site)}>{children}</HomeLayout>;
}
