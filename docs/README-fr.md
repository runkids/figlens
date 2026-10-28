# figlens — Guide utilisateur

[← README](../README-fr.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [Installation autonome](#install)
- [Pourquoi figlens plutôt que le MCP Figma officiel ?](#why)
- [Ce que reçoit l’agent](#bundle)
- [Extension Pi](#pi)
- [Parcourir un design sans Figma](#viewer)
- [Serveur MCP et skill pour agent](#mcp-and-skill)
- [Référence de la CLI](#commands)

<a id="install"></a>

## Installation autonome

**Pas de Node.js ?** Installez le binaire autonome et utilisez `"command": "figlens", "args": ["mcp"]` à la place :

| Plateforme | Commande |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

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
| Fidélité | Parser indépendant avec des [lacunes connues](../README-fr.md#limitations) | Maintenu par Figma |

La lecture d’un `.fig` se fait hors ligne. L’installation et la mise à jour de figlens, les entrées `gs://`, votre fournisseur de modèle et les dépendances de votre projet nécessitent toujours un réseau.

<a id="bundle"></a>

## Ce que reçoit l’agent

Une frame, un zip. Une structure pour construire, des assets à utiliser, une image pour vérifier.

```text
frame.zip
├── context.md                Start here: instructions for your agent
├── design/
│   ├── nodes.json            Layout, text, styles and expanded instances
│   ├── fonts.json            Font faces the visible text uses, with node ids
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

<a id="accuracy"></a>

### Quelle est la proximité avec Figma ?

Mesurée par rapport aux exports JSON, PNG et SVG de Figma lui-même sur **826 frames de production** et cinq fichiers Community (2026-10-22) :

| Contrôle | Résultat |
|---|---|
| Structure des nodes, noms, visibilité, bounds et texte | Identiques dans 771 / 826 frames |
| Image fills d’origine | 2 608 / 2 608 identiques à l’octet près |
| Assets SVG | 5 014 / 5 197 dans le seuil de différence de pixels |
| Bundles de frames Community | 306 / 306 sans asset manquant |

Résultats sur ces fichiers, pas une garantie pour tous les designs. Méthode et seuils : [rapport de précision](../wiki/accuracy.md).

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

![Demandez un écran à Pi, laissez-le trouver le design, construire avec votre stack et vérifier le résultat](../assets/pi-workflow.svg)

Guide complet, commandes et limites : [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Parcourir un design sans Figma

[![Inspecter localement le design MCP Apps, avec son arbre de calques, sa mise en page et les propriétés des composants](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C pour arrêter
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML hors ligne à envoyer à n’importe qui
```

Parcourez les pages et les calques, recherchez, mesurez, inspectez la mise en page, la typographie, les variables et le CSS, exportez n’importe quel calque, copiez un prompt d’implémentation ou téléchargez un bundle pour la sélection. Déposez une capture d’écran de votre build pour la superposer au design. Détails : [guide du preview](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## Serveur MCP et skill pour agent

**MCP.** Le serveur s’exécute en local via stdio et expose `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `verify` et `export`. Passez des chemins absolus. Ajoutez `--root /path/to/designs` (répétable) pour confiner chaque chemin de fichier et de sortie aux dossiers choisis. Les exports raster sont réduits à `maxSize` (1600 px par défaut) pour que les images inline restent légères ; SVG ne renvoie que des métadonnées.

Avec `workflow`, l’agent peut lire les guides intégrés `build`, `audit`, `images`, `plan`, `planning-details` ou `yolo` sur demande explicite. Ils partagent les sources du skill portable ; son installation séparée est facultative avec MCP. L’agent réalise le travail avec ses outils de projet et de navigateur et les permissions de l’hôte. Lire un guide n’exécute pas le workflow ; son utilisation dépend du client et du comportement du modèle.

Windows natif avec Claude Code : suivez ses [notes de configuration stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) et le wrapper `cmd /c` pour `npx`.

**Skill.** Le [skill figlens](../skills/figlens/SKILL.md) apprend à un agent à trouver des frames, lire le bundle par petits morceaux, planifier un travail ciblé et vérifier l’UI en cours d’exécution par rapport au design. Il n’installe que des instructions ; la CLI ou le serveur MCP doit être disponible. Installez-le avec la [Skills CLI](https://github.com/vercel-labs/skills) ou copiez `skills/figlens/` dans le répertoire de skills de votre agent (Claude Code : `~/.claude/skills/`) :

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

**Notes d’installation.** Le lanceur npm télécharge le binaire natif correspondant à la première utilisation, le vérifie avec les checksums SHA-256 de la release et le met en cache. Les installations par script se mettent à jour avec `figlens update`, celles de Homebrew avec `brew upgrade figlens`. Les binaires couvrent macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl statique) et Windows (x86_64).

<a id="commands"></a>

## Référence de la CLI

| Commande | Sortie |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Ids, types et noms des nodes ; texte indenté ou JSON imbriqué |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Recherche insensible à la casse dans les noms de calques et le texte ; chemins, tailles et aperçus. 20 résultats par défaut (1-200) |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID]` | Résumé compact de la mise en page/du texte étendus, 200 nodes au plus. `--images` ajoute les dimensions des bitmaps et les emprises de recadrage |
| `nodes <file> <node>` | JSON au format REST : instances étendues et overrides, auto-layout, styles de texte, fills, effets, variables liées, propriétés de composants, render bounds |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | Un seul décodage, plusieurs nœuds : une ligne JSON par id listé, la sortie de `nodes` avec son `nodeId`, ou `{nodeId, error}` |
| `variables <file> [node]` | Les variables liées d’une frame ; sans node, toutes les variables locales |
| `bundle <file> <node> <out.zip> [--states]` | Un bundle de design pour une frame ; affiche `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | Plusieurs bundles depuis un seul décodage, en parallèle |
| `verify <bundle>` | Vérifie un bundle (`.zip` ou dossier extrait) par rapport à son manifeste : fichiers vérifiés, manquants et modifiés, assets manquants et polices à confirmer dans le projet |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP ou AVIF ; `--max-kb` abaisse la qualité avec perte jusqu’à tenir dans la limite |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local en lecture seule ; `--out` écrit un snapshot hors ligne autonome |
| `compare <file> <nodes.json> <node>` | Diff par rapport à un export REST de la même frame |
| `node <file> <node>` | Un node `.fig` brut |
| `mcp` | Serveur MCP local via stdio |
| `update` | Remplace le binaire par la dernière release |

Pour les variantes de composants liées, utilisez `inspect --states` ou `bundle --states` (MCP : `states: true`, false par défaut ; aucun changement de configuration). Les preuves conservent les sources et les marqueurs de données inconnues ou tronquées. Les bundles ajoutent `design/states.json` et un résumé dans `context.md`. Voir la [preuve des états](../wiki/states.md) pour le périmètre et les limites.

Comparez deux variantes du même component set avec `inspect <file> <variantA> --compare-variant <variantB>` (MCP `compareVariant`). `variantComparison` décrit les différences de champs et ensembles de valeurs stockés avec sources et inconnues, sans apparier les nœuds enfants. Voir les [limites](../wiki/states.md#compare-two-variants).

Un argument node accepte `12:345`, `12-345` ou une URL Figma avec `?node-id=`. `<file>` peut aussi être `gs://bucket/object`, téléchargé avec votre connexion `gcloud` et mis en cache. Consultez le [guide des snapshots GCS](../wiki/gcs.md) pour la transmission via un objet fixe, les droits requis et les limites d’actualisation. Chaque appel de la CLI décode le fichier ; MCP ne réutilise que le dernier fichier local inchangé. Les appels distants relancent l’analyse. Lancez `figlens --help` ou `figlens inspect --help` (`-h` aussi) pour l’aide complète de la CLI.

`inspect` inclut le redimensionnement du texte, la troncature, le nombre maximal de lignes et les décorations enregistrés. `handoffDiagnostics` signale les données sémantiques enregistrées mais non émises, avec des comptes, des exemples et des [limites de couverture](../wiki/semantics.md#known-handoff-gaps). Les sorties protègent la source et remplacent la destination après une écriture complète. MCP limite les lignes de requête à 1 MiB et les réponses complètes à 8 MiB ; un dépassement renvoie une erreur explicite. Le JSON complet reste disponible via la CLI.
