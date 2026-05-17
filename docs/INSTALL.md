# Installation de Kyro sur Windows

## Prérequis

- **Windows 10 64 bits** ou plus récent.
- **mpv.net** installé (gestionnaire de paquets ou [téléchargement officiel](https://github.com/mpvnet-player/mpv.net/releases)). Kyro le détecte automatiquement dans `%PROGRAMFILES%\mpv.net\` et `%LOCALAPPDATA%\mpv.net\`.
- Aucune connexion Internet n'est nécessaire pour faire tourner Kyro.

## Quel artefact télécharger ?

Trois formats sont disponibles dans `dist/` :

| Fichier | Taille | Usage |
|---|---|---|
| `Kyro_1.0.0_x64-setup.exe` | ~44 Mio | **Recommandé.** Installeur NSIS unifié français/anglais, gère l'upgrade in-place. |
| `Kyro_1.0.0_x64_fr-FR.msi` | ~68 Mio | MSI français (déploiement entreprise / Group Policy). |
| `Kyro_1.0.0_x64_en-US.msi` | ~68 Mio | MSI anglais. |
| `Kyro-portable.exe` | ~19 Mio | Binaire seul. À placer manuellement. **Ne contient pas le sidecar `ffprobe.exe`** : à fournir séparément. |

## Vérifier l'intégrité (optionnel mais recommandé)

Un fichier `SHA256SUMS.txt` accompagne chaque release dans `dist/`. Pour vérifier :

```powershell
cd C:\Users\<toi>\Downloads
Get-FileHash -Algorithm SHA256 .\Kyro_1.0.0_x64-setup.exe
```

Compare la valeur affichée avec celle dans `SHA256SUMS.txt`. Elles doivent être identiques caractère pour caractère. Une divergence signifie que le fichier a été modifié ou corrompu pendant le téléchargement.

## L'avertissement SmartScreen Windows

Au premier lancement, Windows affiche probablement :

> **Microsoft Defender SmartScreen a empêché le démarrage d'une application non reconnue. L'exécution de cette application peut mettre votre PC en danger.**

C'est normal pour un installeur **non signé**. Kyro n'a pas de certificat Authenticode (300-500€ par an), ce que SmartScreen interprète comme un drapeau rouge. Le code source est public et le binaire que tu télécharges est exactement celui produit par `cargo tauri build` à partir de ce code source.

Pour passer outre :

1. Cliquer sur **« Informations complémentaires »** dans le dialogue SmartScreen.
2. Cliquer sur **« Exécuter quand même »** qui apparaît alors.

Tu peux aussi débloquer le fichier en amont via PowerShell :

```powershell
Unblock-File -Path .\Kyro_1.0.0_x64-setup.exe
```

Cela retire le marqueur `Zone.Identifier` que Windows attache aux fichiers téléchargés.

## Installation pas à pas

### Via le setup NSIS (recommandé)

1. Double-clic sur `Kyro_1.0.0_x64-setup.exe`.
2. Choisir la langue d'installation (français ou anglais) au premier dialogue.
3. Accepter le chemin par défaut `C:\Program Files\Kyro\` (installation pour tous les utilisateurs).
4. Au premier lancement de Kyro, aller dans **Paramètres → Lecteur externe** pour vérifier que mpv.net a bien été détecté. Sinon, parcourir manuellement.
5. Toujours dans Paramètres : **« Installer le script Lua »** une seule fois, pour activer le suivi de progression mpv.net (la position et le statut « vu » s'auto-mettent à jour).

### Via le MSI (Group Policy / déploiement entreprise)

```powershell
msiexec /i Kyro_1.0.0_x64_fr-FR.msi /qb-
```

Le MSI installe pour la machine entière (`PROGRAMFILES`). Les paramètres et la base de données sont par utilisateur dans `%APPDATA%\Kyro\`.

### Mode portable

1. Crée un dossier `Kyro\` n'importe où (ex. `D:\Apps\Kyro\`).
2. Copie `Kyro-portable.exe` dedans.
3. Procure-toi `ffprobe.exe` ([build statique x64 Windows depuis FFmpeg](https://www.gyan.dev/ffmpeg/builds/)) et place-le à côté du binaire.
4. Lance `Kyro-portable.exe`. Les données seront écrites dans `%APPDATA%\Kyro\` (le mode portable n'est pas encore "stand-alone" complet).

## Mise à jour

Kyro n'a pas de mise à jour automatique (par choix : zéro requête réseau côté Kyro). Pour passer à une version plus récente :

1. Dans l'app : **Paramètres → À propos → « Voir les releases »**. La page GitHub s'ouvre dans le navigateur.
2. Télécharger le nouvel installeur.
3. L'exécuter par-dessus l'installation existante. L'`upgradeCode` est figé donc Windows fait un upgrade in-place propre.

La base de données (`%APPDATA%\Kyro\library.db`) et les affiches importées sont **conservées**.

## Désinstallation

- Setup NSIS : Panneau de configuration → Programmes → Désinstaller un programme → Kyro.
- MSI : `msiexec /x Kyro_1.0.0_x64_fr-FR.msi`.

Pour effacer aussi les données utilisateur (paramètres, progressions) :

```powershell
Remove-Item -Recurse -Force "$env:APPDATA\Kyro"
Remove-Item -Recurse -Force "$env:LOCALAPPDATA\app.kyro"
```

## Dépannage

**« mpv.net introuvable »** dans la barre de notification ou en lançant un fichier : Paramètres → Lecteur externe → renseigner le chemin manuellement (`C:\Program Files\mpv.net\mpv.net.exe` typiquement).

**« ffprobe est introuvable »** (bandeau jaune) : Paramètres → Chemin ffprobe → renseigner. Sans ffprobe, les durées et codecs ne sont pas extraits, mais la lecture fonctionne quand même.

**La progression ne se met pas à jour pendant la lecture** : assure-toi que le bouton « Installer le script Lua » a bien été cliqué dans Paramètres. Le fichier `%APPDATA%\mpv.net\scripts\kyro.lua` doit exister.

**Bibliothèque qui disparaît après un rename de dossier** : c'était un bug en alpha, corrigé par le watcher filesystem de la v1.0. Si ça arrive encore, clique « Rafraîchir » dans la sidebar.

**Logs détaillés** : `%LOCALAPPDATA%\app.kyro\logs\kyro.log.YYYY-MM-DD`. Format JSON ligne, parsable avec `jq` ou `Select-String`.
