import { searchRoute } from '@weebo/operator-docs/routes';
import { source } from '@/lib/source';

// Static search: the index is exported once at build time and queried in the
// browser, so the site needs no server (GitHub Pages).
export const revalidate = false;

export const GET = searchRoute(source);
