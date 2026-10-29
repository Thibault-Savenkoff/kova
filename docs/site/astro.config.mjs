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
      defaultLocale: 'root',
      locales: {
        root: { label: 'English', lang: 'en' },
        fr: { label: 'Français', lang: 'fr' },
      },
      logo: { src: './src/assets/kova.svg' },
      favicon: '/favicon.svg',
      // First visit only: a browser set to French opens the French pages. After that, the
      // language picker decides, so someone who switches back to English stays there.
      head: [{
        tag: 'script',
        content: `try {
  if (!localStorage.getItem('kova-lang-seen')) {
    localStorage.setItem('kova-lang-seen', '1');
    const p = location.pathname;
    if (!p.startsWith('/fr/') && p !== '/fr' && (navigator.language || '').toLowerCase().startsWith('fr')) {
      location.replace('/fr' + p + location.search + location.hash);
    }
  }
} catch (e) {}`,
      }],
      social: [{ icon: 'github', label: 'GitHub', href: 'https://github.com/Thibault-Savenkoff/kova' }],
      expressiveCode: { shiki: { langs: [kova] } },
      sidebar: [
        { label: 'Getting started', translations: { fr: 'Démarrer' }, items: [{ autogenerate: { directory: 'start' } }] },
        { label: 'Language guide', translations: { fr: 'Guide du langage' }, items: [{ autogenerate: { directory: 'guide' } }] },
        { label: 'Library', translations: { fr: 'Bibliothèque' }, items: [{ autogenerate: { directory: 'library' } }] },
        { label: 'Tools', translations: { fr: 'Outils' }, items: [{ autogenerate: { directory: 'tools' } }] },
      ],
    }),
  ],
});
