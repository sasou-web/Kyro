# Notes — Sidecar ffprobe et seuil de taille 50 Mo

Ce document accompagne la sous-tâche **6.3 « Configurer le bundling sidecar de
ffprobe »** du plan d'implémentation (`tasks.md`) et précise la règle de
calcul du seuil de taille de l'installeur Kyro.

## Règle issue de Requirement 1.3

Citation de `requirements.md` (Requirement 1, critère 1.3) :

> THE Kyro SHALL produire un binaire installé dont la taille sur disque est
> inférieure ou égale à 50 Mo, **hors dépendance ffprobe**.

La mention « hors dépendance ffprobe » est explicite : la taille du binaire
ffprobe embarqué en sidecar n'entre pas dans le budget des 50 Mo.

## Application dans Kyro

- `src-tauri/tauri.conf.json` déclare le sidecar :
  ```json
  "bundle": {
    "externalBin": ["binaries/ffprobe"]
  }
  ```
- Tauri attend le fichier `src-tauri/binaries/ffprobe-x86_64-pc-windows-msvc.exe`
  (suffixe de triple cible ajouté automatiquement à `binaries/ffprobe`).
- Build LGPL minimal recommandé (gyan.dev `essentials` ou BtbN `lgpl`) ; le
  binaire pèse environ **~25 Mo**.
- Voir `src-tauri/binaries/README.md` pour la procédure d'obtention et de
  nommage.

## Conséquence pour la mesure de conformité

Pour vérifier la conformité au Requirement 1.3, la taille à mesurer est :

```
taille_installeur_msi_ou_nsis − taille(ffprobe-x86_64-pc-windows-msvc.exe)
```

Cette valeur doit rester **≤ 50 Mo**. La taille brute de l'installeur (avec
ffprobe inclus) sera typiquement de l'ordre de **~75 Mo** une fois le
sidecar ajouté ; cette valeur n'est pas un dépassement du Requirement 1.3.

## Exclusion du dépôt

Le binaire ffprobe **n'est pas committé** :

- `.gitignore` racine ignore `src-tauri/binaries/*.exe`.
- Seuls `src-tauri/binaries/README.md` et `src-tauri/binaries/.gitkeep`
  sont versionnés.
- Le binaire doit être déposé localement par les développeurs (ou par la
  pipeline CI au moment du build) avant `tauri build`.

## Liens croisés

- `requirements.md` — Requirement 1.3, Requirement 5.3.
- `design.md` — section « Stratégie de packaging ffprobe (requirement 5.3,
  5.4) ».
- `tasks.md` — sous-tâche 6.3.
