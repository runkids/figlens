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

[Installation autonome](docs/README-fr.md#install)

Demandez ensuite un écran :

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

L’agent trouve la frame, regroupe les données de design, les assets et une image de référence, puis construit l’écran avec votre stack. Le workflow partagé guide les vérifications des polices et des ressources ; les éléments manquants sont signalés pendant que l’implémentation de la mise en page continue avec des alternatives documentées. Demandez les fichiers modifiés et une comparaison dans le navigateur.

<a id="demo"></a>

## Démo : Pi construit un écran React à partir d’un .fig

[![Pi planifie à partir d’un .fig local, construit un écran React et le vérifie à côté d’un navigateur en direct](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

Le GIF en boucle lit la vidéo montée à une vitesse trois fois supérieure. Cliquez pour voir la vidéo complète de 78 secondes.

**[Vidéo de 78 secondes](assets/taskflow-demo.mp4).** Pi lance `/figlens plan`, lit une frame de dashboard dans un `.fig` local, la construit avec React et Tailwind CSS, puis vérifie et corrige l’UI en cours d’exécution. Time-lapse monté, pas un benchmark ni une promesse de fidélité au pixel près. Design : [Task Flow par Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [notes d’enregistrement](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

**[Du .fig au HTML/CSS modifiable](examples/task-list/README.md) :** dans une étude contrôlée sur une liste de tâches, les six essais basés sur figlens ont conservé les 19 occurrences de texte visible comme véritable texte DOM et les quatre icônes. L’extraction CLI et MCP a également réussi avec le réseau Docker désactivé. L’étude présente la configuration, les mesures et les comparaisons d’images indépendantes.

<a id="why"></a>

## Pourquoi figlens plutôt que le MCP Figma officiel ?

Utilisez **figlens** quand le design doit voyager sous forme de fichier : agents distants, CI, travail hors ligne, passation entre agents, lectures répétées illimitées. Utilisez le **MCP Figma officiel** quand l’agent doit voir le design en direct, sans étape d’export.

La lecture d’un `.fig` se fait hors ligne. L’installation et la mise à jour de figlens, les entrées `gs://`, votre fournisseur de modèle et les dépendances de votre projet nécessitent toujours un réseau.

[Comparaison complète](docs/README-fr.md#why)

<a id="bundle"></a>

## Ce que reçoit l’agent

Une frame, un zip. Une structure pour construire, des assets à utiliser, une image pour vérifier.

[Contenu du bundle et exemples](docs/README-fr.md#bundle) · [Quelle est la proximité avec Figma ?](docs/README-fr.md#accuracy)

## Documentation

- <a id="pi"></a>[Extension Pi](docs/README-fr.md#pi)
- <a id="viewer"></a>[Parcourir un design sans Figma](docs/README-fr.md#viewer)
- <a id="mcp-and-skill"></a>[Serveur MCP et skill pour agent](docs/README-fr.md#mcp-and-skill)
- <a id="commands"></a>[Référence de la CLI](docs/README-fr.md#commands)

<a id="limitations"></a>

## Limites

- **Des snapshots, pas une synchronisation en direct.** Exportez un nouveau `.fig` quand le design change. figlens ne peut pas télécharger un design depuis une URL Figma.
- **Format non documenté.** figlens décode le schéma embarqué dans le fichier ; les nouveaux exports Figma peuvent nécessiter une mise à jour de figlens.
- **Polices et texte :** les fichiers de polices ne sont pas inclus ; un texte avec override peut conserver la mise en page enregistrée par Figma. Utilisez des polices autorisées du projet et comparez le résultat.
- **Couverture :** les [améliorations du rendu](https://github.com/runkids/figlens/issues/36) et la [sémantique de transmission](https://github.com/runkids/figlens/issues/37) sont suivies dans les issues. Consultez la [couverture sémantique actuelle](wiki/semantics.md).
- **Vérifiez la sortie.** Lisez les `warnings` du bundle, `missing-assets.json` et les variables non résolues dans `context.md`, puis comparez l’implémentation au design.
- Les limites de rendu visibles connues figurent dans `_figlens.renderDiagnostics` des nœuds, les résumés bornés de `inspect` et les `warnings` du bundle ; `missing-assets.json` ne contient que les échecs de ressources. L’absence d’avertissements ne garantit pas la fidélité des pixels.

## Aidez figlens à grandir

Si figlens a transformé un design en code pour vous, [mettez une étoile au repo](https://github.com/runkids/figlens) pour que d’autres le trouvent. Un fichier qu’il ne sait pas lire ? [Ouvrez une issue](https://github.com/runkids/figlens/issues) avec la commande et le résultat attendu.

Le développement fait tourner Rust uniquement dans Docker : `docker compose run --rm dev cargo test`. Voir [AGENTS.md](AGENTS.md) et le [wiki](wiki/README.md).

## Licence

[MIT](LICENSE). figlens est un projet indépendant, non affilié à Figma, ni approuvé ou sponsorisé par Figma. Le format `.fig` n’est pas documenté et peut changer ; n’utilisez que des fichiers et des assets que vous avez le droit d’utiliser et de partager.
