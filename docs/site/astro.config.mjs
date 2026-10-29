import { defineConfig } from 'astro/config';
import starlight from '@astrojs/starlight';
import { readFileSync } from 'node:fs';

// Kova's colors come from the editor extension's grammar, so the site and the editor agree.
const kova = JSON.parse(readFileSync(new URL('../../editors/vscode/syntaxes/kova.tmLanguage.json', import.meta.url)));
kova.name = 'kova';

export default defineConfig({
  integrations: [
    starlight({
      title: 'Kova',
      logo: { src: './src/assets/kova.svg' },
      favicon: '/favicon.svg',
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/Thibault-Savenkoff/kova' }],
      expressiveCode: { shiki: { langs: [kova] } },
      sidebar: [
        { label: 'Getting started', items: [{ autogenerate: { directory: 'start' } }] },
        { label: 'Language guide', items: [{ autogenerate: { directory: 'guide' } }] },
        { label: 'Library', items: [{ autogenerate: { directory: 'library' } }] },
        { label: 'Tools', items: [{ autogenerate: { directory: 'tools' } }] },
      ],
    }),
  ],
});
