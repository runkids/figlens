<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Votre design Figma, prêt pour votre agent de développement.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  Donnez à votre agent de développement la mise en page, le texte, les variables, les ressources et une image de référence de n'importe quel écran.<br>
  Lisez-le aussi souvent que vous voulez, sur votre portable, dans le cloud ou en CI.<br>
  <strong>Aucune application Figma, connexion, seat ou limite d'API sur la machine qui travaille.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">Voir la démo</a> · <a href="#quick-start">Essayer votre design</a> · <a href="#for-ai-agents">Connecter votre agent</a> · <a href="#pi-extension">Utiliser avec Pi</a> · <a href="#how-close-is-it-to-figma">Précision</a>
</p>

<a id="why-figlens"></a>

## Pourquoi figlens ?

« Construis cet écran » devrait s'accompagner de sa mise en page, de son texte et de ses ressources. Avec des captures, l'agent doit estimer les espacements et la typographie, et vous exportez toujours les icônes à la main. Le serveur MCP de Figma lit le vrai design, mais son quota appartient à l'équipe propriétaire du fichier, un agent peut épuiser le quota du jour en un seul prompt, et la machine doit être connectée à Figma.

figlens lit le design depuis une copie enregistrée. Dans Figma, choisissez **File > Save local copy** (un accès en lecture suffit, sauf si le propriétaire a désactivé la copie). Donnez ce `.fig` à votre agent avec la CLI ou le serveur MCP local : il peut trouver un écran, inspecter sa mise en page et son texte, et extraire ses ressources. Ou regroupez les données, les ressources et le rendu de référence d'une frame dans un zip utilisable par n'importe quel agent ou collègue.

| Si vous rencontrez ce problème | Avec figlens, vous pouvez |
|---|---|
| « Rate limit exceeded, try again tomorrow », même avec un seat payant | Lire le fichier enregistré autant que nécessaire, sans appel à l'API Figma, jeton ni seat |
| Votre agent s'exécute sur un serveur cloud ou en CI | Lire le design avec une CLI native, sans application Figma ni session de navigateur |
| Vous expliquez sans cesse les mesures et exportez les ressources | Fournir la structure, la typographie, les instances développées, les variables liées et les ressources originales |
| Un autre agent ou collègue a besoin du même écran | Partager un bundle de frame contenant données, ressources, rendu de référence et instructions |

Les designers continuent dans Figma. Votre agent utilise la stack de votre projet. figlens fournit le contexte du design ; l'agent de développement écrit l'implémentation.

<details>
<summary>Le format .fig n'est pas documenté. Que se passe-t-il si Figma le change ?</summary>

figlens ne devine pas le format. Chaque version est vérifiée contre les exports JSON, PNG et SVG de Figma lui-même sur les mêmes fichiers ([résultats](#how-close-is-it-to-figma)) : un changement de format apparaît comme une régression mesurée, pas silencieuse. La copie que vous avez déjà enregistrée continue de fonctionner avec la version de figlens qui l'a lue ; seuls les nouveaux enregistrements depuis un Figma modifié demandent une mise à jour de figlens, que `figlens update` récupère.

</details>

<a id="from-a-local-fig-to-working-html"></a>

## D'un .fig local à une interface HTML fonctionnelle

[![Pi construit une interface depuis un fichier Figma local, avec un aperçu à côté du terminal](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[Voir la vidéo](assets/demo-8x.mp4)** · 45 secondes, **accélérées 8×**. Une vraie session Pi dans Orca : lecture du design local, création du HTML/CSS, puis essai du dialogue de partage et de la zone de saisie dans l'aperçu. L'interface HTML utilise 19 icônes SVG du bundle.

| Cet exemple | Résultat mesuré |
|---|---|
| Fichier `.fig` local | 57.5 MiB |
| Frame sélectionnée | 153 nœuds, 25 ressources SVG, aucune ressource manquante |
| Extraction du bundle | Médiane de **0.47 s** sur cinq exécutions avec cache chaud |

Le chronométrage porte sur `figlens bundle`, **pas sur le temps d'implémentation de l'IA**. Mesuré avec figlens 0.1.2 dans un conteneur Docker ARM64 ; voir les [commandes, résultats et notes d'enregistrement](wiki/accuracy.md#local-design-to-html-demo). Pi écrit le code ; figlens fournit les données. Le fournisseur d'IA utilise toujours le réseau.

<details>
<summary>Crédit du design et limites de la démo</summary>

La frame « List views » provient de [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). La démo utilise la police sans-serif du système et Georgia, car les polices Anthropic originales ne sont pas incluses. Share ouvre un dialogue local ; les réponses restent dans la page et disparaissent au rechargement. L'enregistrement assemble des captures de la vraie fenêtre Orca : les mouvements sont moins fluides qu'une capture vidéo native.

</details>

<a id="quick-start"></a>

## Démarrage rapide

<a id="1-open-your-design-locally"></a>

### 1. Ouvrez votre design localement

Dans Figma, choisissez **File > Save local copy** et enregistrez `design.fig`. Puis exécutez :

```sh
npx @runkids/figlens preview design.fig
```

Ouvrez l'URL locale affichée dans le terminal. Parcourez les pages et sélectionnez l'écran à construire. Inspectez ses espacements, sa typographie, ses variables et ses ressources directement dans l'aperçu.

Node.js 18+ requis. Au premier lancement, le binaire natif est téléchargé, vérifié et mis en cache. Les [installations autonomes](#installation) ne nécessitent ni Node.js, ni Rust, ni Docker. Arrêtez l'aperçu avec Ctrl+C.

<a id="2-give-the-screen-to-your-agent"></a>

### 2. Transmettez l'écran à votre agent

Téléchargez le bundle de la frame sélectionnée depuis l'aperçu. Donnez le zip à votre agent avec cette consigne :

> Extrayez ce bundle dans son propre dossier et lisez context.md. Construisez cet écran avec la stack et les conventions du projet. Utilisez les valeurs de mise en page et les ressources fournies, puis vérifiez le résultat avec references/screen.png. Signalez les ressources manquantes et les polices indisponibles.

Le bundle réunit structure, ressources et image de référence ; l'agent peut le lire avec ses outils de fichiers habituels. Pour qu'il cherche et inspecte directement les écrans, [connectez la CLI ou le serveur MCP](#for-ai-agents). [Avec Pi, demandez un écran par son nom](#pi-extension).

<a id="3-check-the-implementation"></a>

### 3. Vérifiez l'implémentation

Importez une capture de votre implémentation dans l'aperçu. Comparez-la au design côte à côte ou en superposition avec opacité réglable. La capture reste dans votre navigateur. Il s'agit d'une vérification visuelle ; figlens ne calcule pas de score de précision pour votre implémentation.

**Le `.fig` est un instantané.** Enregistrez une nouvelle copie locale quand le design change. Une URL Figma peut sélectionner un nœud de cette copie ; elle ne télécharge pas le design. Après installation, la lecture fonctionne hors ligne ; votre fournisseur d'IA et les dépendances du projet peuvent encore nécessiter le réseau.

<details>
<summary>Vous préférez le terminal ? Trouvez une frame et créez son bundle directement.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Remplacez `12:345` par un identifiant de l'arbre ou des résultats de recherche. Un nœud accepte un identifiant (`12:345`), sa forme URL (`12-345`) ou une URL Figma avec `?node-id=`. `find` cherche les noms et le texte ; `inspect` développe les enfants des instances et résume mise en page et texte. Voir les [commandes](#commands).

</details>

<a id="browse-and-hand-off-a-design"></a>

## Parcourir et transmettre un design

[![Inspection locale du design MCP Apps, avec arbre des calques, mise en page et propriétés des composants](assets/preview-inspector.png)](assets/pi-demo.mp4)

L'aperçu local permet d'explorer un design enregistré sans session Figma :

- Parcourez pages et calques, cherchez noms et texte, mesurez les distances et inspectez mise en page, typographie, variables, propriétés des composants et CSS.
- Exportez un calque en SVG, PNG, JPG, WebP ou AVIF, ou téléchargez ses images de remplissage originales.
- Copiez son identifiant, le contexte structuré ou une consigne d'implémentation ; téléchargez un bundle de la sélection exacte, sous-calques d'instances compris.

Pour envoyer un design consultable à quelqu'un, exportez un instantané HTML autonome :

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

Il s'ouvre sans serveur ni réseau et contient les données, les images et un bundle couvrant l'instantané. Partagez-le uniquement avec les personnes autorisées à voir ces ressources. Sélectionner un enfant change le contexte inspecté ; le bundle intégré garde le périmètre de l'instantané.

L'aperçu surveille le fichier local et se rafraîchit après un enregistrement réussi ; un enregistrement partiel ou échoué signale une vue périmée. Les instantanés HTML restent fixes. Voir le [guide d'aperçu](wiki/preview.md) pour les commandes, le cache, l'automatisation par navigateur et les limites.

<a id="pi-extension"></a>

## Extension Pi

**Donnez un `.fig` à Pi et demandez l'écran.** Le paquet comprend CLI, MCP, aperçu, extension et skill pour agent :

```sh
pi install npm:@runkids/figlens
```

Redémarrez Pi ou lancez `/reload`, puis formulez votre demande normalement :

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi trouve l'écran, lit sa mise en page et ses ressources, inspecte votre projet et utilise sa stack existante. Pas besoin de connaître les identifiants de frames ou de lancer `/figlens open`. Si un nom est ambigu, Pi peut proposer quelques candidats. MCP démarre automatiquement, sans installation globale de la CLI.

![Demandez un écran à Pi : il trouve le design, utilise votre stack et vérifie le résultat](assets/pi-workflow.svg)

**Réutilisation facultative des composants :** demandez à Pi d'enregistrer les correspondances réutilisables, puis acceptez ou modifiez les propositions du projet avec `/figlens review`. Les décisions restent dans `.figlens/component-mappings.md` ; les variantes ou le code enregistré modifiés nécessitent une révision. Ces correspondances ne sont pas nécessaires pour construire un écran.

[![Inspecter le design local, examiner un composant HTML existant dans Pi et le réutiliser dans la frame suivante](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Voir le workflow facultatif de correspondances](assets/pi-demo.mp4)** · Montage de 20 secondes de la v0.3.0 : inspection de MCP Apps, validation d'une correspondance préparée avec du HTML existant, puis réutilisation dans une nouvelle session. La génération du modèle n'est ni montrée ni chronométrée. [Guide, commandes et limites](pi/README.md) · [Notes d'enregistrement et de vérification](wiki/pi.md#demo-and-verification).

Pi 1.3.0+ requis. L'extension npm utilise son lanceur CLI inclus ; le premier lancement, y compris le démarrage automatique de MCP, télécharge et vérifie le binaire natif correspondant. Homebrew et les archives de release fournissent la CLI native et l'aperçu. Pour le développement, voir la [configuration depuis les sources](pi/README.md#install-once).

<a id="the-design-bundle"></a>

## Le bundle de design

Une frame, un zip. La structure pour construire, les ressources à utiliser et une image pour comparer.

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

Besoin d'un seul élément ? Exportez le JSON ou effectuez directement le rendu d'un nœud :

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

L'export SVG accepte frames, groupes, icônes et sous-calques précis d'instances : chaque sous-arbre sélectionné devient un SVG autonome, avec les images originales intégrées et le texte en contours de glyphes. Pas besoin d'extraire un bundle. Le SVG garde ses dimensions ; `--scale` et `--quality` ne concernent que les formats matriciels. Via MCP, les exports matriciels sont réduits pour tenir dans `maxSize` (1600 px sur le plus grand côté par défaut ; 0 le désactive), afin que les images inline restent petites.

<a id="how-close-is-it-to-figma"></a>

## Quelle fidélité à Figma ?

figlens est vérifié avec les exports JSON, PNG et SVG de Figma. Les résultats enregistrés sur **826 frames de production** et **cinq fichiers Community** comprennent :

| Vérification | Résultat |
|---|---|
| Structure, noms, visibilité, limites et texte des nœuds | Identiques sur 771 / 826 frames de production |
| Images de remplissage originales | 2,608 / 2,608 identiques octet par octet |
| Ressources SVG | 5,014 / 5,197 dans le seuil de différence de pixels |
| Bundles de frames Community | 306 / 306 sans ressource manquante |

Ces mesures concernent ces fichiers et ne garantissent pas tous les designs. Voir le [rapport de précision](wiki/accuracy.md) pour les dates, seuils, rendus et méthodes.

**Limites connues :** constraints, layoutAlign/layoutGrow, grilles de mise en page et définitions de propriétés des composants ne sont pas encore émis. Le rendu omet le flou d'arrière-plan et certains réglages d'image (contraste, température, teinte, hautes lumières, ombres), et représente les dégradés angulaires et en losange par des dégradés radiaux. Le texte utilise les contours de glyphes enregistrés, sans fichiers de polices, mais le texte remplacé peut conserver la dernière mise en page de Figma. Les fichiers de polices pour votre implémentation ne sont pas inclus.

<a id="for-ai-agents"></a>

## Pour les agents IA

Utilisez la CLI avec un skill pour agent, ou connectez le même binaire comme serveur MCP.

<a id="agent-skill"></a>

### Skill pour agent

Le [skill figlens](skills/figlens/SKILL.md) apprend à votre agent à trouver les frames, à lire le bundle par petites parties et à utiliser ses valeurs.

Installez-le avec la [Skills CLI](https://github.com/vercel-labs/skills) :

```sh
npx skills add runkids/figlens --skill figlens
```

Pour gérer et mettre à jour les skills dans Claude Code, Codex, Cursor, Gemini CLI et d'autres outils, utilisez [skillshare](https://github.com/runkids/skillshare) :

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` les maintient à jour. Vous pouvez aussi copier `skills/figlens/` dans le dossier de skills de l'agent (`~/.claude/skills/` pour Claude Code). Les archives de release l'incluent aussi.

<a id="mcp-server"></a>

### Serveur MCP

Gérez serveurs MCP et skills avec [skillshare](https://github.com/runkids/skillshare). Définissez figlens une fois et synchronisez sa connexion vers Claude Code, Codex et Cursor :

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

Voir le [guide MCP de Skillshare](https://skillshare.runkids.cc/docs/reference/commands/mcp/) pour les autres clients et l'import de réglages existants.

Pour configurer directement Claude Code :

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

Autres clients MCP :

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

Avec une installation autonome de `figlens`, utilisez :

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Le serveur s'exécute localement via stdio et expose `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` et `export`. Donnez des chemins absolus aux outils : les chemins relatifs partent du dossier où le client a lancé figlens.

Commencez par `find` et `inspect` pour choisir une frame. `export` enregistre le fichier et retourne ses métadonnées. Les formats matriciels retournent aussi du contenu image MCP : l'agent peut voir le design sans ouvrir de fichier local. SVG ne retourne que les métadonnées. PNG offre la compatibilité client la plus large ; réduisez `scale` pour les grandes frames. `find`, `inspect`, `bundle` et `export` retournent aussi du JSON structuré.

## Installation

Utilisez `npx @runkids/figlens` avec Node.js 18+, ou installez la CLI autonome :

| Plateforme | Commande |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Les binaires couvrent macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl statique) et Windows (x86_64 ; ARM64 par émulation). Mettez à jour les installations autonomes avec `figlens update`.

<a id="commands"></a>

## Commandes

| Commande | Sortie |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Identifiants, types et noms ; texte indenté (moins de contexte pour l'agent), ou JSON imbriqué avec `--json` |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Recherche de sous-chaînes sans distinction de casse dans les noms et le texte ; chemins, dimensions et aperçus. 20 résultats par défaut (1–200) ; `truncated` signale la suite |
| `inspect <file> <node> [--depth N]` | Résumé compact de mise en page/texte, instances développées. Profondeur 1 par défaut (0–10), 200 nœuds maximum ; `nodes` pour tout |
| `nodes <file> <node>` | JSON de forme REST : instances et remplacements développés, auto-layout, styles de texte, remplissages, effets, variables liées, propriétés et limites de rendu |
| `variables <file> [node]` | Variables liées à une frame ; sans nœud, toutes les variables locales |
| `bundle <file> <node> <out.zip>` | Bundle d'une frame ; retourne `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Plusieurs bundles en parallèle après un seul décodage ; une ligne JSON par frame, comme ci-dessus ou `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | SVG, PNG, JPG, WebP ou AVIF ; `--max-kb` réduit la qualité avec perte jusqu'à la limite ; PNG/SVG échouent au-delà ; retourne `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Aperçu local en lecture seule ; `--out` exige un nœud et écrit un instantané autonome hors ligne |
| `compare <file> <nodes.json> <node>` | Différences avec l'export REST de la même frame |
| `node <file> <node>` | Un nœud `.fig` brut |
| `mcp` | Serveur MCP local via stdio |
| `update` | Remplace le binaire par la dernière release |

`find` cherche les calques enregistrés et retourne identifiants, chemins des ancêtres, dimensions locales et aperçus du texte. `inspect` développe les instances et limite la sortie à 200 nœuds et 200 caractères par texte ; `truncated` et `textTruncated` signalent les omissions. Chaque appel de la CLI décode le fichier ; le serveur MCP conserve le fichier analysé entre les appels jusqu'à ce qu'il change sur le disque.

`<file>` accepte aussi `gs://bucket/object`, téléchargé avec votre connexion `gcloud` et mis en cache. Lancez `figlens --help` pour l'aide.

<a id="help-figlens-grow"></a>

## Aider figlens à grandir

Si figlens vous aide à transformer votre prochain design en code, [ajoutez une étoile](https://github.com/runkids/figlens) pour le faire connaître. Un design pose problème ? [Ouvrez une issue](https://github.com/runkids/figlens/issues) avec la commande utilisée et le résultat attendu.

En développement, Rust s'exécute uniquement dans Docker : `docker compose run --rm dev cargo test`. `make devc` ouvre un shell où `figlens` utilise les sources de ce checkout ; `make help` liste les cibles du conteneur. Reopen in Container de VS Code utilise le même service. Voir [AGENTS.md](AGENTS.md) pour les règles du projet et le [wiki](wiki/README.md) pour les sujets techniques.

<a id="license"></a>

## Licence

[MIT](LICENSE). figlens est un projet indépendant, sans affiliation, approbation ni parrainage de Figma. Le format `.fig` n'est pas documenté et peut changer ; figlens lit les fichiers enregistrés depuis votre propre compte Figma.
