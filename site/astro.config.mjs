import { defineConfig } from 'astro/config';

// Static output → dist/ ; deploy anywhere (GitHub Pages / nginx / Caddy).
export default defineConfig({
  site: 'https://moodle.ntnu.cc',
  output: 'static',
  trailingSlash: 'ignore',
});
