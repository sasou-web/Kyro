/**
 * @vitest-environment jsdom
 *
 * Tests unitaires de la page `/logs` (tâche 15.8).
 *
 * Couvre :
 *  - au mount, `readLatestLog` est appelé et son contenu est rendu
 *    dans un `<pre>` monospace (requirement 18.3) ;
 *  - le bouton « Recharger » déclenche un nouvel appel et met à jour
 *    le `<pre>` ;
 *  - en cas d'erreur, un message de fallback
 *    « Impossible de lire les logs : … » est affiché à la place du
 *    `<pre>` ;
 *  - quand le log est vide, un message dédié remplace le `<pre>`
 *    (évite d'afficher un bloc vide qui aurait l'air cassé).
 *
 * Le wrapper `readLatestLog` de `$lib/api/commands` est mocké via
 * `vi.hoisted` + `vi.mock`, comme dans `layout.test.ts` (cf.
 * `_Requirements: 18.3_`).
 */

import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/svelte';

const mocks = vi.hoisted(() => ({
  readLatestLog: vi.fn<[], Promise<string>>()
}));

vi.mock('$lib/api/commands', () => ({
  readLatestLog: mocks.readLatestLog
}));

import LogsPage from './+page.svelte';

beforeEach(() => {
  mocks.readLatestLog.mockReset();
});

afterEach(() => {
  cleanup();
});

describe('/logs — chargement initial', () => {
  it('appelle readLatestLog au mount et affiche le contenu dans un <pre>', async () => {
    mocks.readLatestLog.mockResolvedValueOnce('line1\nline2\n');

    const { findByTestId } = render(LogsPage);

    const pre = await findByTestId('logs-content');
    expect(pre.tagName).toBe('PRE');
    expect(pre.textContent).toBe('line1\nline2\n');
    expect(mocks.readLatestLog).toHaveBeenCalledTimes(1);
  });

  it('affiche un message dédié quand le log est vide', async () => {
    mocks.readLatestLog.mockResolvedValueOnce('');

    const { findByTestId, queryByTestId } = render(LogsPage);

    const empty = await findByTestId('logs-empty');
    expect(empty.textContent).toMatch(/aucun log/i);
    expect(queryByTestId('logs-content')).toBeNull();
  });
});

describe('/logs — bouton Recharger', () => {
  it('relance readLatestLog et met à jour le <pre>', async () => {
    mocks.readLatestLog
      .mockResolvedValueOnce('first')
      .mockResolvedValueOnce('second');

    const { findByTestId, getByTestId } = render(LogsPage);

    let pre = await findByTestId('logs-content');
    expect(pre.textContent).toBe('first');

    fireEvent.click(getByTestId('logs-reload'));

    await waitFor(() => {
      pre = getByTestId('logs-content');
      expect(pre.textContent).toBe('second');
    });

    expect(mocks.readLatestLog).toHaveBeenCalledTimes(2);
  });
});

describe('/logs — gestion d\'erreur', () => {
  it('affiche un fallback "Impossible de lire les logs : …" en cas d\'échec', async () => {
    mocks.readLatestLog.mockRejectedValueOnce(new Error('permission denied'));

    const { findByTestId, queryByTestId } = render(LogsPage);

    const err = await findByTestId('logs-error');
    expect(err.textContent).toMatch(/Impossible de lire les logs/);
    expect(err.textContent).toMatch(/permission denied/);
    expect(queryByTestId('logs-content')).toBeNull();
  });
});
