/**
 * @vitest-environment jsdom
 *
 * Tests unitaires du composant `MediaCard.svelte` (tâche 14.1).
 *
 * Vérifient les trois exigences visuelles de la carte :
 *  - le titre est rendu ;
 *  - l'année est rendue si elle est définie ;
 *  - une image (poster) ou un SVG par défaut est rendu selon que
 *    `poster_path` est présent ou non.
 *
 * _Requirements: 15.1, 15.2_
 */

import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, render } from '@testing-library/svelte';

afterEach(() => {
  cleanup();
});

import MediaCard from './MediaCard.svelte';
import type { MediaItem } from '$lib/api/types';

type MediaCardItem = MediaItem & { poster_path?: string | null };

const baseItem: MediaCardItem = {
  id: 1,
  title: 'Blade Runner',
  original_title: null,
  year: 1982,
  type: 'Film',
  status: 'non_vu',
  genres: [],
  poster_path: null
};

describe('MediaCard', () => {
  it('renders the title', () => {
    const { getByText } = render(MediaCard, { props: { item: baseItem } });
    expect(getByText('Blade Runner')).toBeTruthy();
  });

  it('renders the year when defined', () => {
    const { getByText } = render(MediaCard, { props: { item: baseItem } });
    expect(getByText('1982')).toBeTruthy();
  });

  it('omits the year element when year is null', () => {
    const { container } = render(MediaCard, {
      props: { item: { ...baseItem, year: null } }
    });
    expect(container.querySelector('.year')).toBeNull();
  });

  it('renders the local poster <img> via convertFileSrc when poster_path is set', () => {
    const { container } = render(MediaCard, {
      props: { item: { ...baseItem, poster_path: 'C:\\posters\\42.jpg' } }
    });

    const img = container.querySelector('img');
    expect(img).not.toBeNull();
    // `convertFileSrc` produit une URL `http://asset.localhost/...` côté
    // Windows en environnement test (le mock de l'IPC Tauri retombe
    // sur le préfixe sans encoder le chemin). On vérifie juste que le
    // chemin source apparaît dans le src final, et qu'aucune URL
    // brute disque n'est passée à `<img>`.
    const src = img?.getAttribute('src') ?? '';
    expect(src.length).toBeGreaterThan(0);
    expect(src.startsWith('C:\\')).toBe(false);
    expect(src).toMatch(/asset|localhost|posters/i);
    expect(img?.getAttribute('alt')).toBe('Blade Runner');
    // Pas de SVG placeholder quand on a une image.
    expect(container.querySelector('svg.poster-placeholder')).toBeNull();
  });

  it('renders an inline SVG placeholder when poster_path is missing', () => {
    const { container } = render(MediaCard, {
      props: { item: { ...baseItem, poster_path: null } }
    });

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('svg.poster-placeholder')).not.toBeNull();
  });

  it('treats an empty poster_path string as missing and falls back to the SVG', () => {
    const { container } = render(MediaCard, {
      props: { item: { ...baseItem, poster_path: '' } }
    });

    expect(container.querySelector('img')).toBeNull();
    expect(container.querySelector('svg.poster-placeholder')).not.toBeNull();
  });
});
