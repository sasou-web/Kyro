# Requirements Document

## Introduction

Kyro est une application Windows légère, construite avec Tauri (backend Rust, frontend Web), qui sert de bibliothèque personnelle pour films, séries et animes stockés localement. L'application offre une expérience visuelle façon mini Netflix/Plex, avec lancement direct des fichiers dans mpv.net et reprise de lecture exacte via un canal IPC entre mpv et Kyro.

Toutes les métadonnées sont gérées localement et éditées manuellement par l'utilisateur (aucun scraping en ligne). L'application extrait automatiquement les informations techniques (codec, durée, résolution, pistes audio/sous-titres) via ffprobe et déduit le titre, l'année, la qualité et la numérotation d'épisode à partir du nom de fichier au moyen d'expressions régulières.

L'interface adopte un thème sombre moderne inspiré de Adwaita Dark / Libadwaita Dark, avec une palette gris-bleu sombre, un accent bleu profond, des cards arrondies et une esthétique desktop calme et premium.

## Glossary

- **Kyro**: Application desktop Windows construite avec Tauri (backend Rust, frontend Web) et objet de la présente spécification.
- **Kyro_Backend**: Composant Rust de Kyro, responsable du système de fichiers, du scan, du parsing, de l'IPC et de la persistance.
- **Kyro_Frontend**: Composant Web (HTML/CSS/JS) de Kyro, responsable du rendu de l'interface utilisateur.
- **Kyro_Library**: Base de données locale persistante (SQLite ou équivalent) contenant les fiches Films, Séries, Animes, Saisons, Épisodes, ainsi que leur progression de lecture, statut vu/non vu, favoris et liste à regarder.
- **Kyro_Scanner**: Sous-composant de Kyro_Backend qui parcourt les dossiers configurés par l'utilisateur pour découvrir les fichiers vidéo.
- **Kyro_Parser**: Sous-composant de Kyro_Backend qui extrait, à partir du nom de fichier, le titre, l'année, la qualité et la numérotation d'épisode au moyen d'expressions régulières.
- **Kyro_Probe**: Sous-composant de Kyro_Backend qui invoque ffprobe pour extraire les métadonnées techniques d'un fichier vidéo.
- **Kyro_Player_Bridge**: Sous-composant de Kyro_Backend qui lance le lecteur externe configuré et gère le canal IPC avec mpv.net.
- **Kyro_IPC_Pipe**: Canal de communication local (named pipe Windows) utilisé entre Kyro_Player_Bridge et le script Lua exécuté dans mpv.net.
- **Kyro_Lua_Script**: Script Lua chargé dans mpv.net qui pousse la position de lecture courante et l'événement de fermeture vers Kyro_IPC_Pipe.
- **Kyro_Theme**: Thème graphique par défaut de Kyro_Frontend, inspiré de Adwaita Dark / Libadwaita Dark.
- **Media_Item**: Entrée unique de Kyro_Library représentant un Film, une Série ou un Anime.
- **Episode_Item**: Entrée de Kyro_Library représentant un épisode appartenant à une Série ou un Anime.
- **Watch_Progress**: Position de lecture en secondes (entier non négatif) associée à un Media_Item ou à un Episode_Item.
- **Supported_Video_Extensions**: Ensemble des extensions reconnues par Kyro_Scanner: `.mkv`, `.mp4`, `.avi`, `.mov`, `.webm`, `.m4v`, `.ts`, `.wmv`.
- **External_Player**: Application externe configurée par l'utilisateur pour lire les fichiers vidéo, mpv.net étant la valeur par défaut.

## Requirements

### Requirement 1: Application desktop Tauri Windows

**User Story:** En tant qu'utilisateur Windows, je veux une application desktop légère qui démarre rapidement, afin de parcourir ma bibliothèque média sans surcharge système.

#### Acceptance Criteria

1. THE Kyro SHALL être distribuée sous forme d'un exécutable Windows construit avec Tauri, combinant un Kyro_Backend en Rust et un Kyro_Frontend Web.
2. WHEN l'utilisateur lance Kyro, THE Kyro SHALL afficher la fenêtre principale en moins de 3 secondes sur un poste équipé d'un SSD et de 8 Go de RAM.
3. THE Kyro SHALL produire un binaire installé dont la taille sur disque est inférieure ou égale à 50 Mo, hors dépendance ffprobe.
4. IF la fenêtre principale ne peut être créée au démarrage, THEN THE Kyro SHALL écrire un message d'erreur dans le journal local et terminer le processus avec un code de sortie non nul.

### Requirement 2: Configuration des dossiers de bibliothèque

**User Story:** En tant qu'utilisateur, je veux déclarer les dossiers locaux qui contiennent mes médias, afin que Kyro sache où chercher.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL fournir une page de paramètres permettant d'ajouter, de retirer et de lister les dossiers de bibliothèque.
2. WHEN l'utilisateur ajoute un dossier de bibliothèque, THE Kyro_Backend SHALL persister le chemin absolu du dossier dans Kyro_Library.
3. WHEN l'utilisateur retire un dossier de bibliothèque, THE Kyro_Backend SHALL supprimer ce chemin de Kyro_Library et marquer les Media_Item rattachés à ce dossier comme indisponibles.
4. IF un chemin ajouté n'existe pas sur le système de fichiers au moment de l'ajout, THEN THE Kyro_Backend SHALL refuser l'ajout et retourner un message d'erreur identifiant le chemin invalide.
5. THE Kyro_Backend SHALL associer chaque dossier de bibliothèque à une catégorie parmi `Films`, `Séries`, `Animes`.

### Requirement 3: Scan automatique des dossiers

**User Story:** En tant qu'utilisateur, je veux que Kyro découvre automatiquement les fichiers vidéo de mes dossiers, afin de ne pas saisir mes médias manuellement.

#### Acceptance Criteria

1. WHEN l'utilisateur déclenche un scan, THE Kyro_Scanner SHALL parcourir récursivement chaque dossier de bibliothèque configuré.
2. THE Kyro_Scanner SHALL ne retenir que les fichiers dont l'extension appartient à Supported_Video_Extensions.
3. WHEN Kyro_Scanner découvre un fichier déjà présent dans Kyro_Library avec le même chemin absolu et la même taille, THE Kyro_Scanner SHALL conserver l'entrée existante sans dupliquer.
4. WHEN Kyro_Scanner découvre un fichier non encore présent dans Kyro_Library, THE Kyro_Scanner SHALL créer une nouvelle entrée Media_Item ou Episode_Item selon la catégorie du dossier parent.
5. WHEN un fichier précédemment indexé n'est plus présent sur disque lors d'un scan, THE Kyro_Scanner SHALL marquer l'entrée correspondante comme indisponible sans la supprimer.
6. WHILE un scan est en cours, THE Kyro_Frontend SHALL afficher un indicateur de progression précisant le nombre de fichiers traités sur le total détecté.
7. WHERE l'option `scan_au_démarrage` est activée dans les paramètres, THE Kyro SHALL lancer un scan automatique au démarrage de l'application.

### Requirement 4: Parsing du nom de fichier

**User Story:** En tant qu'utilisateur, je veux que Kyro déduise automatiquement le titre, l'année, la qualité et la numérotation d'épisode depuis le nom de fichier, afin d'éviter la saisie manuelle initiale.

#### Acceptance Criteria

1. THE Kyro_Parser SHALL extraire le titre en supprimant les délimiteurs `.`, `_`, `-` et en retirant les segments de qualité et de numérotation reconnus.
2. THE Kyro_Parser SHALL extraire l'année lorsqu'une séquence à 4 chiffres comprise entre 1900 et 2099 est présente dans le nom de fichier.
3. THE Kyro_Parser SHALL extraire la qualité parmi `480p`, `720p`, `1080p`, `1440p`, `2160p`, `4K` lorsqu'elle apparaît dans le nom de fichier.
4. WHEN le nom de fichier contient un motif de la forme `S<saison>E<épisode>` (insensible à la casse), THE Kyro_Parser SHALL extraire les numéros de saison et d'épisode comme entiers.
5. WHEN le nom de fichier contient un motif de la forme `<saison>x<épisode>`, THE Kyro_Parser SHALL extraire les numéros de saison et d'épisode comme entiers.
6. WHEN le nom de fichier contient un motif `ép. <numéro>`, `ep <numéro>`, `episode <numéro>` ou `- <numéro>` (insensible à la casse), THE Kyro_Parser SHALL extraire le numéro d'épisode et associer la saison `1` par défaut.
7. IF aucun motif de numérotation reconnu n'est détecté pour un fichier rangé dans une catégorie `Séries` ou `Animes`, THEN THE Kyro_Parser SHALL marquer l'Episode_Item comme `numérotation_inconnue` et l'associer à un dossier `À classer` dans Kyro_Library.

### Requirement 5: Extraction des métadonnées techniques via ffprobe

**User Story:** En tant qu'utilisateur, je veux voir la durée, le codec, la résolution et les pistes audio/sous-titres de mes fichiers, afin de connaître leurs caractéristiques techniques sans les ouvrir.

#### Acceptance Criteria

1. WHEN Kyro_Scanner ajoute un nouveau Media_Item ou Episode_Item, THE Kyro_Probe SHALL invoquer ffprobe sur le fichier pour extraire la durée en secondes, le codec vidéo, la résolution en pixels (largeur x hauteur), la liste des pistes audio (codec et langue) et la liste des pistes de sous-titres (codec et langue).
2. THE Kyro_Probe SHALL persister la taille du fichier en octets dans Kyro_Library.
3. THE Kyro_Backend SHALL embarquer ou détecter automatiquement un binaire ffprobe utilisable sur Windows.
4. IF le binaire ffprobe est introuvable au démarrage, THEN THE Kyro_Frontend SHALL afficher un message d'avertissement décrivant l'absence de ffprobe et indiquant comment le configurer dans les paramètres.
5. IF l'invocation de ffprobe échoue pour un fichier, THEN THE Kyro_Probe SHALL conserver l'entrée dans Kyro_Library avec les métadonnées techniques marquées `non disponibles` et journaliser l'erreur.
6. WHEN l'utilisateur déclenche une réanalyse d'un Media_Item ou Episode_Item, THE Kyro_Probe SHALL relancer ffprobe et mettre à jour les champs techniques correspondants.

### Requirement 6: Édition manuelle des fiches

**User Story:** En tant qu'utilisateur, je veux éditer manuellement les fiches de mes médias (titre, synopsis, genres, note, affiche locale), afin de personnaliser ma bibliothèque sans dépendre d'Internet.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL fournir un formulaire d'édition pour chaque Media_Item permettant de modifier le titre, le titre original, l'année, le synopsis, les genres, la note utilisateur (entier de 0 à 10), le studio ou réalisateur, et le statut.
2. THE Kyro_Frontend SHALL permettre à l'utilisateur d'associer une affiche locale à un Media_Item en sélectionnant un fichier image depuis le système de fichiers.
3. WHEN l'utilisateur enregistre une fiche éditée, THE Kyro_Backend SHALL persister les valeurs dans Kyro_Library et copier l'image d'affiche sélectionnée dans le répertoire de données local de Kyro.
4. THE Kyro_Backend SHALL ne jamais émettre de requête réseau sortante pour récupérer ou enrichir des métadonnées de Media_Item ou Episode_Item.
5. WHERE un Media_Item est de type `Série` ou `Anime`, THE Kyro_Frontend SHALL permettre l'édition des fiches au niveau de la série, de chaque saison et de chaque Episode_Item.
6. IF l'utilisateur saisit une note utilisateur hors de l'intervalle 0 à 10, THEN THE Kyro_Frontend SHALL refuser l'enregistrement et afficher un message indiquant la plage autorisée.

### Requirement 7: Catégories Films, Séries et Animes

**User Story:** En tant qu'utilisateur, je veux trois catégories distinctes Films, Séries et Animes, afin de naviguer rapidement entre mes types de contenus.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL afficher une navigation principale exposant trois catégories : `Films`, `Séries`, `Animes`.
2. WHEN l'utilisateur sélectionne une catégorie, THE Kyro_Frontend SHALL afficher uniquement les Media_Item rattachés à cette catégorie.
3. WHERE un Media_Item est de type `Série` ou `Anime`, THE Kyro_Library SHALL regrouper les Episode_Item associés sous des saisons numérotées.
4. WHEN l'utilisateur ouvre un Media_Item de type `Série` ou `Anime`, THE Kyro_Frontend SHALL afficher la liste des saisons et, pour chaque saison, la liste ordonnée des Episode_Item.

### Requirement 8: Suivi vu / non vu

**User Story:** En tant qu'utilisateur, je veux marquer mes médias comme vus ou non vus, afin de suivre ce qu'il me reste à regarder.

#### Acceptance Criteria

1. THE Kyro_Library SHALL associer à chaque Media_Item de type `Film` et à chaque Episode_Item un statut parmi `non_vu`, `en_cours`, `vu`.
2. WHEN la valeur de Watch_Progress d'un Media_Item ou Episode_Item dépasse 90 % de sa durée totale, THE Kyro_Backend SHALL passer son statut à `vu`.
3. WHEN la valeur de Watch_Progress d'un Media_Item ou Episode_Item est strictement supérieure à 0 et inférieure ou égale à 90 % de sa durée totale, THE Kyro_Backend SHALL passer son statut à `en_cours`.
4. THE Kyro_Frontend SHALL permettre à l'utilisateur de forcer manuellement le statut d'un Media_Item ou Episode_Item à `vu` ou `non_vu`.
5. WHERE un Media_Item est de type `Série` ou `Anime`, THE Kyro_Frontend SHALL afficher le nombre d'Episode_Item vus sur le total d'Episode_Item de la série.

### Requirement 9: Reprise de lecture exacte via mpv.net

**User Story:** En tant qu'utilisateur, je veux reprendre la lecture d'un film ou d'un épisode exactement où je l'ai arrêté, afin de ne pas perdre ma position.

#### Acceptance Criteria

1. WHEN l'utilisateur lance la lecture d'un Media_Item ou Episode_Item, THE Kyro_Player_Bridge SHALL lancer External_Player avec le chemin du fichier et l'argument permettant de positionner la lecture à la valeur courante de Watch_Progress.
2. WHILE External_Player est mpv.net et que Kyro_Lua_Script est chargé, THE Kyro_Lua_Script SHALL pousser la position de lecture courante vers Kyro_IPC_Pipe au moins une fois toutes les 5 secondes.
3. WHEN External_Player se ferme, THE Kyro_Lua_Script SHALL pousser un message final contenant la dernière position de lecture connue vers Kyro_IPC_Pipe.
4. WHEN Kyro_Player_Bridge reçoit une mise à jour de position via Kyro_IPC_Pipe, THE Kyro_Backend SHALL mettre à jour Watch_Progress du Media_Item ou Episode_Item concerné dans Kyro_Library.
5. IF Kyro_IPC_Pipe ne reçoit aucun message pendant la session de lecture, THEN THE Kyro_Backend SHALL conserver la valeur de Watch_Progress antérieure sans la modifier.
6. THE Kyro_IPC_Pipe SHALL utiliser un named pipe Windows dédié dont le nom inclut un identifiant de session unique généré par Kyro_Player_Bridge à chaque lancement.
7. THE Kyro_Player_Bridge SHALL fournir une procédure d'installation de Kyro_Lua_Script dans le dossier de scripts de mpv.net depuis les paramètres de Kyro.

### Requirement 10: Choix du lecteur externe

**User Story:** En tant qu'utilisateur, je veux pouvoir choisir mon lecteur vidéo externe, afin d'utiliser le lecteur de mon choix tout en conservant mpv.net par défaut.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL exposer dans les paramètres un champ permettant de sélectionner le chemin de l'exécutable External_Player.
2. THE Kyro SHALL définir mpv.net comme valeur par défaut de External_Player lors de la première installation.
3. WHEN l'utilisateur sélectionne un exécutable pour External_Player, THE Kyro_Backend SHALL vérifier que le fichier existe et qu'il est exécutable, puis persister le chemin dans Kyro_Library.
4. IF le chemin de External_Player configuré n'existe plus au moment d'un lancement de lecture, THEN THE Kyro_Frontend SHALL afficher un message d'erreur invitant l'utilisateur à reconfigurer le lecteur.
5. WHERE External_Player n'est pas mpv.net, THE Kyro_Player_Bridge SHALL lancer le lecteur avec le chemin du fichier sans tenter d'utiliser Kyro_IPC_Pipe et signaler dans la fiche du Media_Item que la reprise exacte n'est pas garantie.

### Requirement 11: Favoris et liste à regarder

**User Story:** En tant qu'utilisateur, je veux marquer des médias comme favoris ou les ajouter à une liste à regarder, afin d'organiser mes priorités.

#### Acceptance Criteria

1. THE Kyro_Library SHALL associer à chaque Media_Item un indicateur booléen `favori` et un indicateur booléen `à_regarder`.
2. THE Kyro_Frontend SHALL fournir un contrôle pour basculer l'indicateur `favori` d'un Media_Item depuis la vue détaillée et depuis la grille.
3. THE Kyro_Frontend SHALL fournir un contrôle pour basculer l'indicateur `à_regarder` d'un Media_Item depuis la vue détaillée et depuis la grille.
4. THE Kyro_Frontend SHALL exposer une vue dédiée listant les Media_Item dont `favori` est vrai.
5. THE Kyro_Frontend SHALL exposer une vue dédiée listant les Media_Item dont `à_regarder` est vrai.

### Requirement 12: Reprise du dernier épisode regardé

**User Story:** En tant qu'utilisateur d'une série ou d'un anime, je veux reprendre directement au dernier épisode que je regardais, afin de ne pas chercher manuellement où j'en suis.

#### Acceptance Criteria

1. THE Kyro_Library SHALL mémoriser, pour chaque Media_Item de type `Série` ou `Anime`, la référence du dernier Episode_Item dont le statut est `en_cours` ou, à défaut, du premier Episode_Item dont le statut est `non_vu` selon l'ordre saison puis épisode.
2. WHEN l'utilisateur ouvre un Media_Item de type `Série` ou `Anime`, THE Kyro_Frontend SHALL afficher un bouton `Reprendre` qui cible l'Episode_Item mémorisé selon le critère ci-dessus.
3. WHEN tous les Episode_Item d'un Media_Item de type `Série` ou `Anime` ont le statut `vu`, THE Kyro_Frontend SHALL afficher un bouton `Revoir depuis le début` à la place de `Reprendre`.
4. WHEN l'utilisateur active le bouton `Reprendre`, THE Kyro_Player_Bridge SHALL lancer la lecture de l'Episode_Item ciblé en respectant la valeur courante de Watch_Progress.

### Requirement 13: Recherche rapide

**User Story:** En tant qu'utilisateur, je veux rechercher un média par son titre, afin de le trouver rapidement.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL fournir un champ de recherche accessible depuis toutes les vues principales.
2. WHEN l'utilisateur saisit du texte dans le champ de recherche, THE Kyro_Frontend SHALL filtrer les Media_Item affichés dont le titre ou le titre original contient la chaîne saisie, en insensible à la casse et aux accents.
3. THE Kyro_Frontend SHALL mettre à jour les résultats de recherche dans un délai inférieur ou égal à 200 ms après chaque frappe pour une bibliothèque comportant jusqu'à 5000 Media_Item.
4. WHERE la chaîne de recherche est vide, THE Kyro_Frontend SHALL afficher la liste complète de la vue courante sans filtrage.

### Requirement 14: Filtres par genre, année, type et statut

**User Story:** En tant qu'utilisateur, je veux filtrer ma bibliothèque par genre, année, type et statut, afin d'affiner ma navigation.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL fournir des contrôles de filtrage permettant de filtrer par genre, par année, par type (`Film`, `Série`, `Anime`) et par statut (`non_vu`, `en_cours`, `vu`).
2. WHEN l'utilisateur applique un filtre, THE Kyro_Frontend SHALL afficher uniquement les Media_Item satisfaisant simultanément l'ensemble des filtres actifs.
3. THE Kyro_Frontend SHALL permettre de cumuler plusieurs valeurs pour le filtre genre, l'union des valeurs sélectionnées étant retenue.
4. WHEN l'utilisateur retire tous les filtres actifs, THE Kyro_Frontend SHALL afficher la liste complète de la vue courante.
5. THE Kyro_Frontend SHALL afficher en permanence le nombre de Media_Item visibles après application des filtres.

### Requirement 15: Bibliothèque visuelle avec affiches

**User Story:** En tant qu'utilisateur, je veux voir ma bibliothèque sous forme de grille d'affiches, afin de retrouver mes médias visuellement.

#### Acceptance Criteria

1. THE Kyro_Frontend SHALL afficher chaque Media_Item sous la forme d'une carte contenant l'affiche locale, le titre et l'année.
2. WHERE aucun fichier d'affiche n'est associé à un Media_Item, THE Kyro_Frontend SHALL afficher un visuel par défaut intégré à l'application.
3. THE Kyro_Frontend SHALL disposer les cartes en grille adaptative dont le nombre de colonnes s'ajuste à la largeur de la fenêtre.
4. WHEN l'utilisateur survole une carte, THE Kyro_Frontend SHALL afficher la note utilisateur, la durée totale ou le nombre d'épisodes, et le statut courant.

### Requirement 16: Thème par défaut Adwaita Dark

**User Story:** En tant qu'utilisateur, je veux une interface sombre calme et premium inspirée de Adwaita Dark, afin de profiter d'une lecture confortable et d'une esthétique cohérente avec un desktop moderne.

#### Acceptance Criteria

1. THE Kyro_Theme SHALL être appliqué par défaut au premier démarrage de Kyro.
2. THE Kyro_Theme SHALL définir la couleur de fond principale `--bg-primary` à `#1e1e2e`.
3. THE Kyro_Theme SHALL définir la couleur de fond secondaire `--bg-secondary` à `#242535`.
4. THE Kyro_Theme SHALL définir la couleur de surface des cartes `--surface` à `#2a2b3d`.
5. THE Kyro_Theme SHALL définir la couleur des bordures discrètes `--border` à `#383a4f`.
6. THE Kyro_Theme SHALL définir la couleur du texte principal `--text-primary` à `#e6e6ec`.
7. THE Kyro_Theme SHALL définir la couleur du texte secondaire `--text-secondary` à `#a8a8b5`.
8. THE Kyro_Theme SHALL définir la couleur d'accent `--accent` à `#3584e4`.
9. THE Kyro_Theme SHALL définir la couleur d'accent au survol `--accent-hover` à `#1c71d8`.
10. THE Kyro_Theme SHALL appliquer un rayon de coin `--radius-card` égal à 12 pixels sur les cartes de Media_Item.
11. THE Kyro_Theme SHALL appliquer un rayon de coin `--radius-control` égal à 8 pixels sur les boutons et champs de saisie.
12. THE Kyro_Theme SHALL utiliser une famille de polices sans empattement de type `Inter` ou la police système Windows par défaut en repli.
13. THE Kyro_Theme SHALL exclure toute couleur d'accent violette ou néon, l'accent restant dans la gamme bleue définie ci-dessus.
14. WHERE l'utilisateur souhaite ajuster le thème, THE Kyro_Frontend SHALL exposer dans les paramètres l'option de basculer entre Kyro_Theme et un thème clair équivalent inspiré de Adwaita.

### Requirement 17: Persistance locale et confidentialité

**User Story:** En tant qu'utilisateur, je veux que toutes mes données restent locales, afin de préserver ma vie privée et de pouvoir utiliser Kyro hors ligne.

#### Acceptance Criteria

1. THE Kyro_Library SHALL être stockée localement dans le répertoire de données utilisateur de Kyro sur le poste Windows.
2. THE Kyro_Backend SHALL persister toutes les modifications de Media_Item, Episode_Item et Watch_Progress dans Kyro_Library avant de signaler l'opération comme réussie au Kyro_Frontend.
3. THE Kyro_Backend SHALL n'émettre aucune requête réseau sortante hors action explicite et future de l'utilisateur.
4. WHEN l'utilisateur désinstalle Kyro, THE Kyro SHALL conserver Kyro_Library dans le répertoire de données utilisateur sauf si l'utilisateur demande explicitement la suppression des données.

### Requirement 18: Journalisation et gestion des erreurs

**User Story:** En tant qu'utilisateur, je veux que Kyro reste utilisable en cas d'erreur isolée, afin de ne pas perdre l'accès à ma bibliothèque pour un fichier problématique.

#### Acceptance Criteria

1. WHEN une opération de scan, de parsing, de probe ou d'IPC échoue pour un fichier, THE Kyro_Backend SHALL journaliser l'erreur avec le chemin du fichier concerné et continuer le traitement des autres fichiers.
2. THE Kyro_Backend SHALL écrire les journaux dans un fichier rotatif situé dans le répertoire de données utilisateur de Kyro.
3. THE Kyro_Frontend SHALL fournir une vue permettant d'accéder au journal le plus récent depuis les paramètres.
4. IF une erreur fatale empêche Kyro_Backend de répondre aux requêtes de Kyro_Frontend, THEN THE Kyro_Frontend SHALL afficher un message indiquant l'état dégradé et proposer de relancer l'application.
