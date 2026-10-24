<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Tu coding agent lee archivos de Figma.<br>Sin app de Figma. Sin login. Sin cuota de API.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://www.npmjs.com/package/@runkids/figlens"><img src="https://img.shields.io/npm/dm/@runkids/figlens?label=npm%20downloads&amp;color=8b78c8" alt="npm downloads per month"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">Empezar</a> · <a href="#demo">Demo</a> · <a href="#why">Por qué</a> · <a href="#bundle">Qué recibe el agente</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limitaciones</a>
</p>

**El problema.** Tu agente necesita valores de layout, texto y recursos para implementar una pantalla. En un servidor remoto o en CI, ese contexto debe estar disponible junto al código sin depender de una aplicación de escritorio en ejecución.

**La solución.** En Figma, **File > Save local copy**. Coloca `design.fig` junto a tu código. figlens lo analiza en local y le da a tu agent layout, texto, estilos, variables, iconos, imágenes y un render de referencia de cualquier frame. Un binario nativo, cero llamadas de red a Figma.

<a id="start"></a>

## Primeros pasos

Pon `design.fig` en tu proyecto y elige tu agent.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Inicia una sesión nueva y comprueba `/mcp`. Para el flujo de planificación guiada, [añade el skill](#mcp-and-skill).

**Pi** (1.3.0+): un solo paquete incluye la CLI, MCP, el viewer y el skill.

```sh
pi install npm:@runkids/figlens
```

**Cualquier cliente MCP** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**¿Sin Node.js?** Instala el binario independiente y usa `"command": "figlens"` en su lugar:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Luego pide una pantalla:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

El agent encuentra el frame, genera un bundle con datos del diseño, recursos y una imagen de referencia, e implementa la pantalla con tu stack. Pídele que muestre los archivos modificados y la comparación; un bundle por sí solo no demuestra que la UI sea correcta.

<a id="demo"></a>

## Demo: Pi construye una pantalla React desde un .fig

[![Pi plans from a local .fig, builds a React screen and checks it beside a live browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

El GIF en bucle reproduce el vídeo editado a triple velocidad. Haz clic para ver el vídeo completo de 78 segundos.

**[Video de 78 segundos](assets/taskflow-demo.mp4).** Pi ejecuta `/figlens plan`, lee un frame de dashboard de un `.fig` local, lo construye con React y Tailwind CSS, y luego revisa y corrige la UI en ejecución. Time-lapse editado, no un benchmark ni una promesa de fidelidad pixel-perfect. Diseño: [Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [notas de la grabación](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

<a id="why"></a>

## ¿Por qué figlens en lugar del MCP oficial de Figma?

Usa **figlens** cuando el diseño deba viajar como archivo: agents remotos, CI, trabajo sin conexión, traspaso entre agents, lecturas repetidas sin límite. Usa el **MCP oficial de Figma** cuando el agent deba ver el diseño en vivo sin un paso de exportación.

| | figlens | MCP oficial de Figma |
|---|---|---|
| Fuente | Snapshot `.fig` exportado | Archivos de Figma en vivo |
| Requiere | El archivo | Inicio de sesión en Figma; el servidor de escritorio requiere la app |
| Límites de uso | Ninguno, análisis local | Límites del servicio de Figma |
| Agent remoto / CI | Copia el archivo, ejecuta el binario | Servidor remoto con inicio de sesión |
| Cambios en el diseño | Exportar un nuevo `.fig` | Automático |
| Traspaso | Compartir el `.fig` o un bundle de frame | Compartir el acceso a Figma |
| Fidelidad | Parser independiente con [límites conocidos](#limitations) | Mantenido por Figma |

Leer un `.fig` funciona sin conexión. Instalar y actualizar figlens, las entradas `gs://`, tu proveedor de modelos y las dependencias de tu proyecto sí necesitan red.

<a id="bundle"></a>

## Qué recibe el agent

Un frame, un zip. Estructura para construir, recursos para usar, una imagen para comprobar.

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

¿Necesitas una sola pieza? La CLI lo hace directamente:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # ids de frame por nombre o texto
npx -y @runkids/figlens inspect design.fig 12:345                 # resumen compacto de layout y texto
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON con forma REST
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP o AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # el zip de arriba
```

Las exportaciones SVG incrustan los image fills originales y convierten el texto en contornos de glifos, así que no hacen falta fuentes. A través de MCP, las exportaciones raster también se devuelven como imágenes inline, de modo que el agent ve el diseño sin abrir un archivo.

### ¿Qué tan cerca está de Figma?

Medido contra las exportaciones JSON, PNG y SVG de la propia Figma en **826 frames de producción** y cinco archivos de la Community (2026-10-22):

| Comprobación | Resultado |
|---|---|
| Estructura de nodos, nombres, visibilidad, bounds y texto | Idénticos en 771 / 826 frames |
| Image fills originales | 2,608 / 2,608 idénticos byte a byte |
| Recursos SVG | 5,014 / 5,197 dentro del umbral de diferencia de píxeles |
| Bundles de frames de la Community | 306 / 306 sin recursos faltantes |

Resultados sobre esos archivos, no una garantía para todos los diseños. Método y umbrales: [informe de precisión](wiki/accuracy.md).

<a id="pi"></a>

## Extensión de Pi

```sh
pi install npm:@runkids/figlens
```

Reinicia Pi o ejecuta `/reload`, y luego pide una pantalla con tus propias palabras. Pi encuentra el frame, lo lee a través del servidor MCP registrado automáticamente y construye con tu stack. Sin ids de nodo, sin CLI global.

- `/figlens plan <request>` solo crea un plan; no modifica la aplicación.
- `/figlens yolo <request>` planifica, construye, comprueba y limpia su propio plan temporal.
- `/figlens preview [description]` abre el viewer local.
- `/figlens review` aprueba los mapeos de componentes reutilizables que propuso el agent.

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](assets/pi-workflow.svg)

Guía completa, comandos y límites: [pi/README.md](pi/README.md).

<a id="viewer"></a>

## Explora un diseño sin Figma

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C para detener
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML offline para enviar a cualquiera
```

Explora páginas y capas, busca, mide, inspecciona layout, tipografía, variables y CSS, exporta cualquier capa, copia un prompt de implementación o descarga un bundle de la selección. Suelta una captura de pantalla de tu build para superponerla al diseño. Detalles: [guía de preview](wiki/preview.md).

<a id="mcp-and-skill"></a>

## Servidor MCP y skill para agents

**MCP.** El servidor se ejecuta en local por stdio y expone `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` y `export`. Pasa rutas absolutas. Añade `--root /path/to/designs` (repetible) para limitar todas las rutas de archivos y de salida a las carpetas elegidas. Las exportaciones raster se reducen a `maxSize` (por defecto 1600 px) para que las imágenes inline sean pequeñas; SVG devuelve solo metadatos.

Windows nativo con Claude Code: sigue sus [notas de configuración stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) y el wrapper `cmd /c` para `npx`.

**Skill.** El [skill de figlens](skills/figlens/SKILL.md) enseña a un agent a encontrar frames, leer el bundle en piezas pequeñas, planificar trabajo acotado y verificar la UI en ejecución contra el diseño. Instala solo instrucciones; la CLI o el servidor MCP deben estar disponibles. Instálalo con la [Skills CLI](https://github.com/vercel-labs/skills) o copia `skills/figlens/` en el directorio de skills de tu agent (Claude Code: `~/.claude/skills/`):

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** Gestiona el skill y la conexión MCP en Claude Code, Codex y Cursor desde un solo lugar con [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**Notas de instalación.** El launcher de npm descarga el binario nativo correspondiente en el primer uso, lo verifica con los checksums SHA-256 de la release y lo guarda en caché. Las instalaciones independientes se actualizan con `figlens update`. Los binarios cubren macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) y Windows (x86_64).

<a id="commands"></a>

## Referencia de la CLI

| Comando | Salida |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Ids, tipos y nombres de nodos; texto con sangría o JSON anidado |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Búsqueda sin distinguir mayúsculas en nombres de capas y texto; rutas, tamaños y vistas previas. 20 resultados por defecto (1-200) |
| `inspect <file> <node> [--depth N] [--images]` | Resumen compacto de layout/texto expandido, como máximo 200 nodos. `--images` añade dimensiones de bitmap y áreas de recorte |
| `nodes <file> <node>` | JSON con forma REST: instancias expandidas y overrides, auto-layout, estilos de texto, fills, efectos, variables enlazadas, propiedades de componentes, render bounds |
| `variables <file> [node]` | Las variables enlazadas de un frame; sin nodo, todas las variables locales |
| `bundle <file> <node> <out.zip>` | Un bundle de diseño para un frame; imprime `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Muchos bundles a partir de una sola decodificación, en paralelo |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP o AVIF; `--max-kb` reduce la calidad con pérdida hasta que quepa |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local de solo lectura; `--out` escribe un snapshot offline independiente |
| `compare <file> <nodes.json> <node>` | Diff contra una exportación REST del mismo frame |
| `node <file> <node>` | Un nodo `.fig` sin procesar |
| `mcp` | Servidor MCP local por stdio |
| `update` | Reemplaza el binario por la última release |

Un argumento de nodo acepta `12:345`, `12-345` o una URL de Figma con `?node-id=`. `<file>` también puede ser `gs://bucket/object`, descargado con tu login de `gcloud` y guardado en caché. Cada llamada a la CLI decodifica el archivo; el servidor MCP conserva el archivo analizado entre llamadas hasta que cambia en disco. Ejecuta `figlens --help` para ver el uso.

<a id="limitations"></a>

## Limitaciones

- **Snapshots, no sincronización en vivo.** Exporta un nuevo `.fig` cuando cambie el diseño. figlens no puede descargar un diseño desde una URL de Figma.
- **Formato no documentado.** figlens decodifica el esquema incrustado en el archivo; las nuevas exportaciones de Figma pueden requerir una actualización de figlens.
- **Aún no se emiten:** anotaciones de Dev Mode, constraints, layoutAlign/layoutGrow, layout grids, definiciones de propiedades de componentes.
- **Huecos de render:** se omiten el desenfoque de fondo y algunos ajustes de imagen; los gradientes angulares y de diamante se dibujan como radiales. El texto con overrides puede conservar el último layout de Figma. No se incluyen los archivos de fuentes.
- **Revisa la salida.** Lee los `warnings` del bundle, `missing-assets.json` y las variables sin resolver en `context.md`, y luego compara la implementación con el diseño.

## Ayuda a que figlens crezca

Si figlens convirtió un diseño en código para ti, [dale una estrella al repo](https://github.com/runkids/figlens) para que otros lo encuentren. ¿Encontraste un archivo que no puede leer? [Abre un issue](https://github.com/runkids/figlens/issues) con el comando y lo que esperabas.

El desarrollo ejecuta Rust solo en Docker: `docker compose run --rm dev cargo test`. Consulta [AGENTS.md](AGENTS.md) y el [wiki](wiki/README.md).

## Licencia

[MIT](LICENSE). figlens es un proyecto independiente, sin afiliación, respaldo ni patrocinio de Figma. El formato `.fig` no está documentado y puede cambiar; usa solo archivos y recursos a los que tengas permiso de acceder y compartir.
