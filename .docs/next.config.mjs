import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  serverExternalPackages: ['@takumi-rs/image-response'],
  output: 'standalone',
  reactStrictMode: true,
  basePath: '/ProxyAuthK8S',
};

export default withMDX(config);
