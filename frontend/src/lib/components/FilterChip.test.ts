/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de `<FilterChip>`.
 *
 * Le composant est volontairement « bête » : il affiche son libellé,
 * reflète l'état `selected` via une classe CSS et `aria-pressed`, et
 * dispatche un événement `toggle` au clic. Aucune logique de store
 * n'est testée ici (cf. `FilterPanel.test.ts`).
 *
 * _Requirements: 14.1, 14.2, 14.3_
 */

import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render } from '@testing-library/svelte';

import FilterChip from './FilterChip.svelte';

afterEach(() => {
  cleanup();
});

describe('<FilterChip>', () => {
  it('affiche le libellé fourni en prop', () => {
    const { getByRole } = render(FilterChip, { props: { label: 'Drame' } });
    const button = getByRole('button', { name: 'Drame' });
    expect(button).toBeTruthy();
  });

  it('expose aria-pressed=false et n\'a pas la classe selected par défaut', () => {
    const { getByRole } = render(FilterChip, { props: { label: 'Drame' } });
    const button = getByRole('button');
    expect(button.getAttribute('aria-pressed')).toBe('false');
    expect(button.classList.contains('selected')).toBe(false);
  });

  it('expose aria-pressed=true et la classe selected quand selected=true', () => {
    const { getByRole } = render(FilterChip, {
      props: { label: 'Drame', selected: true }
    });
    const button = getByRole('button');
    expect(button.getAttribute('aria-pressed')).toBe('true');
    expect(button.classList.contains('selected')).toBe(true);
  });

  it('dispatche un événement `toggle` (sans payload) au clic', async () => {
    const onToggle = vi.fn();
    const { getByRole, component } = render(FilterChip, {
      props: { label: 'Drame' }
    });
    component.$on('toggle', onToggle);

    await fireEvent.click(getByRole('button'));

    expect(onToggle).toHaveBeenCalledTimes(1);
    // CustomEvent.detail vaut `null` (et non `undefined`) lorsque le
    // dispatcher Svelte est appelé sans payload — c'est la valeur par
    // défaut de `CustomEventInit.detail` côté plateforme.
    expect(onToggle.mock.calls[0][0].detail).toBeNull();
  });

  it('dispatche un événement `toggle` à chaque clic (toggle illimité)', async () => {
    const onToggle = vi.fn();
    const { getByRole, component } = render(FilterChip, {
      props: { label: 'Drame' }
    });
    component.$on('toggle', onToggle);

    const button = getByRole('button');
    await fireEvent.click(button);
    await fireEvent.click(button);
    await fireEvent.click(button);

    expect(onToggle).toHaveBeenCalledTimes(3);
  });
});
