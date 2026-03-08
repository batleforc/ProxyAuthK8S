import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
const config = {
  serverExternalPackages: ['@takumi-rs/image-response'],
  output: 'export',
  reactStrictMode: true,
  basePath: '/ProxyAuthK8S',
};

export default withMDX(config);
