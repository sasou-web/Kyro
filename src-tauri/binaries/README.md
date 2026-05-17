# Sidecar `ffprobe` — Kyro

Ce dossier héberge le binaire ffprobe **sidecar** que Tauri embarque dans
l'installeur Kyro et copie à côté de l'exécutable installé. Il est référencé
dans `src-tauri/tauri.conf.json` sous `bundle.externalBin = ["binaries/ffprobe"]`.

## Nom de fichier attendu

Tauri ajoute automatiquement le suffixe correspondant au triple cible Rust à
l'entrée `externalBin`. Pour la cible Windows x64 supportée par Kyro, le
fichier doit donc s'appeler exactement :

```
ffprobe-x86_64-pc-windows-msvc.exe
```

Sans ce nom exact, `tauri build` échoue avec une erreur du type
`failed to find sidecar binary`.

## Source recommandée (build LGPL minimal)

ffmpeg est diffusé sous LGPL/GPL ; pour rester en LGPL et éviter d'embarquer
des composants GPL non requis par Kyro, utiliser un build « LGPL » officiel
plutôt qu'un build complet :

- Builds Windows officiels de gyan.dev :
  https://www.gyan.dev/ffmpeg/builds/ — choisir la variante
  `ffmpeg-release-essentials` (LGPL) puis n'extraire que `bin/ffprobe.exe`.
- Alternative : builds BtbN (https://github.com/BtbN/FFmpeg-Builds), variante
  `ffmpeg-master-latest-win64-lgpl` — extraire `bin/ffprobe.exe`.

Renommer ensuite le binaire extrait en `ffprobe-x86_64-pc-windows-msvc.exe`
et le déposer dans ce dossier.

Kyro n'utilise ffprobe qu'en lecture de métadonnées avec ces arguments :

```
ffprobe -v error -print_format json -show_format -show_streams <fichier>
```

Aucun encodeur ni mux n'est requis ; un build LGPL minimal suffit
fonctionnellement.

## Mentions LGPL / attribution

Si Kyro est redistribué avec ce sidecar, inclure dans la documentation
l'attribution LGPL conforme (lien vers les sources ffmpeg utilisées et
mention de la licence LGPL v2.1+).

## Le binaire n'est pas committé

Le binaire `ffprobe-*.exe` est **exclu du dépôt** via la règle
`src-tauri/binaries/*.exe` dans `.gitignore` à la racine. Seuls ce
`README.md` et un fichier `.gitkeep` sont versionnés.

Le binaire est attendu :

- soit déposé manuellement par les développeurs en local,
- soit récupéré au moment du build CI (étape de pipeline non couverte par
  cette tâche).

## Pourquoi ce sidecar n'entre pas dans la limite 50 Mo

Le Requirement 1.3 du spec Kyro impose un installeur ≤ 50 Mo « hors
dépendance ffprobe ». Le binaire ffprobe LGPL pèse environ 25 Mo et est
exclu de ce calcul. Voir `.kiro/specs/kyro/SIDECAR-NOTES.md` pour le
détail de cette exclusion.
