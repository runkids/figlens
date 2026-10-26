<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Votre coding agent lit les fichiers Figma.<br>Sans app Figma. Sans connexion. Sans quota d’API.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="Licence MIT"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total des téléchargements des assets de release GitHub"></a>
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI et MCP pour agents IA"></a>
</p>

<p align="center">
  <a href="#start">Démarrer</a> · <a href="#demo">Démo</a> · <a href="#why">Pourquoi</a> · <a href="#bundle">Ce que reçoit l’agent</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limites</a>
</p>

**Le problème.** Votre agent a besoin des valeurs de mise en page, du texte et des ressources pour réaliser un écran. Sur une machine distante ou en CI, ce contexte doit être disponible à côté du code, sans dépendre d’une application desktop en cours d’exécution.

**La solution.** Dans Figma, **File > Save local copy**. Placez `design.fig` à côté de votre code. figlens l’analyse en local et donne à votre agent la mise en page, le texte, les styles, les variables, les icônes, les images et un rendu de référence de n’importe quelle frame. Un seul binaire natif, aucun appel réseau vers Figma.

<a id="start"></a>

## Bien démarrer

**Pas encore de fichier de design ?** [Vérifiez votre installation avec le petit exemple public](examples/first-success/README.md) : vérifiez le binaire choisi, trouvez un frame et validez son bundle. Aucun modèle nécessaire.

Placez `design.fig` dans votre projet, puis choisissez votre agent.

**Claude Code** (Node.js 18+, macOS/Linux) :

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Démarrez une nouvelle session et vérifiez `/mcp`. Décrivez l’écran avec vos propres mots : MCP fournit à la demande des guides de planification, d’implémentation et de préparation des images ; aucune installation séparée du skill n’est nécessaire.

**Pi** (1.3.0+) : un seul paquet fournit la CLI, MCP, le viewer et le skill.

```sh
pi install npm:@runkids/figlens
```

**Tout client MCP** (Cursor, Codex, Gemini CLI, ...) :

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**Pas de Node.js ?** Installez le binaire autonome et utilisez `"command": "figlens"` à la place :

| Plateforme | Commande |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Demandez ensuite un écran :

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

L’agent trouve la frame, construit un bundle avec les données de design, les assets et une image de référence, puis implémente l’écran avec votre stack. Demandez-lui d’afficher les fichiers modifiés et la comparaison ; un bundle seul ne prouve pas que l’UI est correcte.

<a id="demo"></a>

## Démo : Pi construit un écran React à partir d’un .fig

[![Pi planifie à partir d’un .fig local, construit un écran React et le vérifie à côté d’un navigateur en direct](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

Le GIF en boucle lit la vidéo montée à une vitesse trois fois supérieure. Cliquez pour voir la vidéo complète de 78 secondes.

**[Vidéo de 78 secondes](assets/taskflow-demo.mp4).** Pi lance `/figlens plan`, lit une frame de dashboard dans un `.fig` local, la construit avec React et Tailwind CSS, puis vérifie et corrige l’UI en cours d’exécution. Time-lapse monté, pas un benchmark ni une promesse de fidélité au pixel près. Design : [Task Flow par Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [notes d’enregistrement](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

<a id="why"></a>

## Pourquoi figlens plutôt que le MCP Figma officiel ?

Utilisez **figlens** quand le design doit voyager sous forme de fichier : agents distants, CI, travail hors ligne, passation entre agents, lectures répétées illimitées. Utilisez le **MCP Figma officiel** quand l’agent doit voir le design en direct, sans étape d’export.

| | figlens | MCP Figma officiel |
|---|---|---|
| Source | Snapshot `.fig` exporté | Fichiers Figma en direct |
| Prérequis | Le fichier | Connexion à Figma ; le serveur desktop nécessite l’application |
| Limites de débit | Aucune, analyse locale | Limites du service Figma |
| Agent distant / CI | Copier le fichier, lancer le binaire | Serveur distant avec connexion |
| Modifications du design | Exporter un nouveau `.fig` | Automatique |
| Passation | Partager le `.fig` ou un bundle de frame | Partager l’accès Figma |
| Fidélité | Parser indépendant avec des [lacunes connues](#limitations) | Maintenu par Figma |

La lecture d’un `.fig` se fait hors ligne. L’installation et la mise à jour de figlens, les entrées `gs://`, votre fournisseur de modèle et les dépendances de votre projet nécessitent toujours un réseau.

<a id="bundle"></a>

## Ce que reçoit l’agent

Une frame, un zip. Une structure pour construire, des assets à utiliser, une image pour vérifier.

```text
frame.zip
├── context.md                Start here: instructions for your agent
├── design/
│   ├── nodes.json            Layout, text, styles and expanded instances
│   └── variables.json        Bound variables, collections, modes and aliases
├── assets/                   Image fills, SVGs and composited node renders
├── assets.json               Maps design references to asset files
├── references/screen.png     A render of the frame to compare against
├── missing-assets.json       Anything that could not be produced
└── manifest.json             File roles, media types, sizes and SHA-256 hashes
```

Besoin d’un seul élément ? La CLI le fait directement :

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # ids de frames par nom ou texte
npx -y @runkids/figlens inspect design.fig 12:345                 # résumé compact de la mise en page et du texte
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON au format REST
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP ou AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # le zip ci-dessus
```

Les exports SVG intègrent les image fills d’origine et convertissent le texte en glyphes vectoriels, donc aucune police n’est nécessaire. Via MCP, les exports raster reviennent aussi en images inline : l’agent voit le design sans ouvrir de fichier.

### Quelle est la proximité avec Figma ?

Mesurée par rapport aux exports JSON, PNG et SVG de Figma lui-même sur **826 frames de production** et cinq fichiers Community (2026-10-22) :

| Contrôle | Résultat |
|---|---|
| Structure des nodes, noms, visibilité, bounds et texte | Identiques dans 771 / 826 frames |
| Image fills d’origine | 2 608 / 2 608 identiques à l’octet près |
| Assets SVG | 5 014 / 5 197 dans le seuil de différence de pixels |
| Bundles de frames Community | 306 / 306 sans asset manquant |

Résultats sur ces fichiers, pas une garantie pour tous les designs. Méthode et seuils : [rapport de précision](wiki/accuracy.md).

<a id="pi"></a>

## Extension Pi

```sh
pi install npm:@runkids/figlens
```

Redémarrez Pi ou lancez `/reload`, puis demandez un écran en langage courant. Pi trouve la frame, la lit via le serveur MCP enregistré automatiquement et construit avec votre stack. Pas d’ids de nodes, pas de CLI globale.

- `/figlens plan <request>` rédige uniquement un plan, sans modifier l’application.
- `/figlens yolo <request>` planifie, construit, vérifie et nettoie son propre plan temporaire.
- `/figlens preview [description]` ouvre le viewer local.
- `/figlens review` approuve les mappings de composants réutilisables proposés par l’agent.

![Demandez un écran à Pi, laissez-le trouver le design, construire avec votre stack et vérifier le résultat](assets/pi-workflow.svg)

Guide complet, commandes et limites : [pi/README.md](pi/README.md).

<a id="viewer"></a>

## Parcourir un design sans Figma

[![Inspecter localement le design MCP Apps, avec son arbre de calques, sa mise en page et les propriétés des composants](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C pour arrêter
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML hors ligne à envoyer à n’importe qui
```

Parcourez les pages et les calques, recherchez, mesurez, inspectez la mise en page, la typographie, les variables et le CSS, exportez n’importe quel calque, copiez un prompt d’implémentation ou téléchargez un bundle pour la sélection. Déposez une capture d’écran de votre build pour la superposer au design. Détails : [guide du preview](wiki/preview.md).

<a id="mcp-and-skill"></a>

## Serveur MCP et skill pour agent

**MCP.** Le serveur s’exécute en local via stdio et expose `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` et `export`. Passez des chemins absolus. Ajoutez `--root /path/to/designs` (répétable) pour confiner chaque chemin de fichier et de sortie aux dossiers choisis. Les exports raster sont réduits à `maxSize` (1600 px par défaut) pour que les images inline restent légères ; SVG ne renvoie que des métadonnées.

Avec `workflow`, l’agent peut lire les guides intégrés `build`, `images`, `plan`, `planning-details` ou `yolo` sur demande explicite. Ils partagent les sources du skill portable ; son installation séparée est facultative avec MCP. L’agent réalise le travail avec ses outils de projet et de navigateur et les permissions de l’hôte. Lire un guide n’exécute pas le workflow ; son utilisation dépend du client et du comportement du modèle.

Windows natif avec Claude Code : suivez ses [notes de configuration stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) et le wrapper `cmd /c` pour `npx`.

**Skill.** Le [skill figlens](skills/figlens/SKILL.md) apprend à un agent à trouver des frames, lire le bundle par petits morceaux, planifier un travail ciblé et vérifier l’UI en cours d’exécution par rapport au design. Il n’installe que des instructions ; la CLI ou le serveur MCP doit être disponible. Installez-le avec la [Skills CLI](https://github.com/vercel-labs/skills) ou copiez `skills/figlens/` dans le répertoire de skills de votre agent (Claude Code : `~/.claude/skills/`) :

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** Gérez le skill et la connexion MCP pour Claude Code, Codex et Cursor depuis un seul endroit avec [skillshare](https://github.com/runkids/skillshare) :

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**Notes d’installation.** Le lanceur npm télécharge le binaire natif correspondant à la première utilisation, le vérifie avec les checksums SHA-256 de la release et le met en cache. Les installations autonomes se mettent à jour avec `figlens update`. Les binaires couvrent macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl statique) et Windows (x86_64).

<a id="commands"></a>

## Référence de la CLI

| Commande | Sortie |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Ids, types et noms des nodes ; texte indenté ou JSON imbriqué |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Recherche insensible à la casse dans les noms de calques et le texte ; chemins, tailles et aperçus. 20 résultats par défaut (1-200) |
| `inspect <file> <node> [--depth N] [--images] [--states]` | Résumé compact de la mise en page/du texte étendus, 200 nodes au plus. `--images` ajoute les dimensions des bitmaps et les emprises de recadrage |
| `nodes <file> <node>` | JSON au format REST : instances étendues et overrides, auto-layout, styles de texte, fills, effets, variables liées, propriétés de composants, render bounds |
| `variables <file> [node]` | Les variables liées d’une frame ; sans node, toutes les variables locales |
| `bundle <file> <node> <out.zip> [--states]` | Un bundle de design pour une frame ; affiche `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | Plusieurs bundles depuis un seul décodage, en parallèle |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP ou AVIF ; `--max-kb` abaisse la qualité avec perte jusqu’à tenir dans la limite |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local en lecture seule ; `--out` écrit un snapshot hors ligne autonome |
| `compare <file> <nodes.json> <node>` | Diff par rapport à un export REST de la même frame |
| `node <file> <node>` | Un node `.fig` brut |
| `mcp` | Serveur MCP local via stdio |
| `update` | Remplace le binaire par la dernière release |

Pour les variantes de composants liées, utilisez `inspect --states` ou `bundle --states` (MCP : `states: true`, false par défaut ; aucun changement de configuration). Les preuves conservent les sources et les marqueurs de données inconnues ou tronquées. Les bundles ajoutent `design/states.json` et un résumé dans `context.md`. Voir la [preuve des états](wiki/states.md) pour le périmètre et les limites.

Un argument node accepte `12:345`, `12-345` ou une URL Figma avec `?node-id=`. `<file>` peut aussi être `gs://bucket/object`, téléchargé avec votre connexion `gcloud` et mis en cache. Consultez le [guide des snapshots GCS](wiki/gcs.md) pour la transmission via un objet fixe, les droits requis et les limites d’actualisation. Chaque appel de la CLI décode le fichier ; MCP ne réutilise que le dernier fichier local inchangé. Les appels distants relancent l’analyse. Lancez `figlens --help` pour l’aide.

<a id="limitations"></a>

## Limites

- **Des snapshots, pas une synchronisation en direct.** Exportez un nouveau `.fig` quand le design change. figlens ne peut pas télécharger un design depuis une URL Figma.
- **Format non documenté.** figlens décode le schéma embarqué dans le fichier ; les nouveaux exports Figma peuvent nécessiter une mise à jour de figlens.
- **Pas encore émis :** annotations du Dev Mode, layout grids.
- **Sémantique partielle :** les constraints, layoutAlign/layoutGrow et définitions de propriétés enregistrés sont émis avec leur provenance dans `_figlens.semantics`. Les valeurs absentes ne sont pas complétées ; les options variant et SLOT ne sont pas pris en charge. Ces correspondances n’ont pas été comparées aux exports Figma de la même version. Voir la [couverture](wiki/semantics.md).
- **Lacunes de rendu :** le background blur et certains réglages d’image sont ignorés ; les dégradés angulaires et en losange sont dessinés comme radiaux. Un texte avec override peut conserver la dernière mise en page de Figma. Les fichiers de polices ne sont pas inclus.
- **Vérifiez la sortie.** Lisez les `warnings` du bundle, `missing-assets.json` et les variables non résolues dans `context.md`, puis comparez l’implémentation au design.
- Les limites de rendu visibles connues figurent dans `_figlens.renderDiagnostics` des nœuds, les résumés bornés de `inspect` et les `warnings` du bundle ; `missing-assets.json` ne contient que les échecs de ressources. L’absence d’avertissements ne garantit pas la fidélité des pixels.

## Aidez figlens à grandir

Si figlens a transformé un design en code pour vous, [mettez une étoile au repo](https://github.com/runkids/figlens) pour que d’autres le trouvent. Un fichier qu’il ne sait pas lire ? [Ouvrez une issue](https://github.com/runkids/figlens/issues) avec la commande et le résultat attendu.

Le développement fait tourner Rust uniquement dans Docker : `docker compose run --rm dev cargo test`. Voir [AGENTS.md](AGENTS.md) et le [wiki](wiki/README.md).

## Licence

[MIT](LICENSE). figlens est un projet indépendant, non affilié à Figma, ni approuvé ou sponsorisé par Figma. Le format `.fig` n’est pas documenté et peut changer ; n’utilisez que des fichiers et des assets que vous avez le droit d’utiliser et de partager.
