# figlens — Guía de uso

[← README](../README-es.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [Instalación independiente](#install)
- [¿Por qué figlens en lugar del MCP oficial de Figma?](#why)
- [Qué recibe el agent](#bundle)
- [Extensión de Pi](#pi)
- [Explora un diseño sin Figma](#viewer)
- [Servidor MCP y skill para agents](#mcp-and-skill)
- [Referencia de la CLI](#commands)

<a id="install"></a>

## Instalación independiente

**¿Sin Node.js?** Instala el binario independiente y usa `"command": "figlens", "args": ["mcp"]` en su lugar:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

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
| Fidelidad | Parser independiente con [límites conocidos](../README-es.md#limitations) | Mantenido por Figma |

Leer un `.fig` funciona sin conexión. Instalar y actualizar figlens, las entradas `gs://`, tu proveedor de modelos y las dependencias de tu proyecto sí necesitan red.

<a id="bundle"></a>

## Qué recibe el agent

Un frame, un zip. Estructura para construir, recursos para usar, una imagen para comprobar.

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

¿Necesitas una sola pieza? La CLI lo hace directamente:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # ids de frame por nombre o texto
npx -y @runkids/figlens inspect design.fig 12:345                 # resumen compacto de layout y texto
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON con forma REST
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP o AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # el zip de arriba
```

Las exportaciones SVG incrustan los image fills originales y convierten el texto en contornos de glifos, así que no hacen falta fuentes. A través de MCP, las exportaciones raster también se devuelven como imágenes inline, de modo que el agent ve el diseño sin abrir un archivo.

<a id="accuracy"></a>

### ¿Qué tan cerca está de Figma?

Medido contra las exportaciones JSON, PNG y SVG de la propia Figma en **826 frames de producción** y cinco archivos de la Community (2026-10-22):

| Comprobación | Resultado |
|---|---|
| Estructura de nodos, nombres, visibilidad, bounds y texto | Idénticos en 771 / 826 frames |
| Image fills originales | 2,608 / 2,608 idénticos byte a byte |
| Recursos SVG | 5,014 / 5,197 dentro del umbral de diferencia de píxeles |
| Bundles de frames de la Community | 306 / 306 sin recursos faltantes |

Resultados sobre esos archivos, no una garantía para todos los diseños. Método y umbrales: [informe de precisión](../wiki/accuracy.md).

<a id="pi"></a>

## Extensión de Pi

```sh
pi install npm:@runkids/figlens
```

Guías de instalación: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

Reinicia Pi o ejecuta `/reload`, y luego pide una pantalla con tus propias palabras. Pi encuentra el frame, lo lee a través del servidor MCP registrado automáticamente y construye con tu stack. Sin ids de nodo, sin CLI global.

- `/figlens plan <request>` solo crea un plan; no modifica la aplicación.
- `/figlens yolo <request>` planifica, construye, comprueba y limpia su propio plan temporal.
- `/figlens preview [description]` abre el viewer local.
- `/figlens review` aprueba los mapeos de componentes reutilizables que propuso el agent.

![Ask Pi for a screen, let it find the design, build with your stack, and check the result](../assets/pi-workflow.svg)

Guía completa, comandos y límites: [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Explora un diseño sin Figma

[![Inspect the MCP Apps design locally, with its layer tree, layout and component properties](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C para detener
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML offline para enviar a cualquiera
```

Explora páginas y capas, busca, mide, inspecciona layout, tipografía, variables y CSS, exporta cualquier capa, copia un prompt de implementación o descarga un bundle de la selección. Suelta una captura de pantalla de tu build para superponerla al diseño. Detalles: [guía de preview](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## Servidor MCP y skill para agents

**MCP.** El servidor se ejecuta en local por stdio y expone `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `verify` y `export`. Pasa rutas absolutas. Añade `--root /path/to/designs` (repetible) para limitar todas las rutas de archivos y de salida a las carpetas elegidas. Las exportaciones raster se reducen a `maxSize` (por defecto 1600 px) para que las imágenes inline sean pequeñas; SVG devuelve solo metadatos.

Con `workflow`, el agent puede leer las guías integradas `build`, `audit`, `images`, `plan`, `planning-details` o `yolo` cuando se solicite expresamente. Comparten las fuentes del skill portable; instalarlo aparte es opcional para MCP. El agent realiza el trabajo con sus herramientas de proyecto y navegador y los permisos del host. Leer una guía no ejecuta el flujo; su uso depende del cliente y del comportamiento del modelo.

Windows nativo con Claude Code: sigue sus [notas de configuración stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) y el wrapper `cmd /c` para `npx`.

**Skill.** El [skill de figlens](../skills/figlens/SKILL.md) enseña a un agent a encontrar frames, leer el bundle en piezas pequeñas, planificar trabajo acotado y verificar la UI en ejecución contra el diseño. Instala solo instrucciones; la CLI o el servidor MCP deben estar disponibles. Instálalo con la [Skills CLI](https://github.com/vercel-labs/skills) o copia `skills/figlens/` en el directorio de skills de tu agent (Claude Code: `~/.claude/skills/`):

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

**Notas de instalación.** El launcher de npm descarga el binario nativo correspondiente en el primer uso, lo verifica con los checksums SHA-256 de la release y lo guarda en caché. Las instalaciones por script se actualizan con `figlens update`; las de Homebrew, con `brew upgrade figlens`. Los binarios cubren macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) y Windows (x86_64).

<a id="commands"></a>

## Referencia de la CLI

| Comando | Salida |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Ids, tipos y nombres de nodos; texto con sangría o JSON anidado |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Búsqueda sin distinguir mayúsculas en nombres de capas y texto; rutas, tamaños y vistas previas. 20 resultados por defecto (1-200) |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID]` | Resumen compacto de layout/texto expandido, como máximo 200 nodos. `--images` añade dimensiones de bitmap y áreas de recorte |
| `nodes <file> <node>` | JSON con forma REST: instancias expandidas y overrides, auto-layout, estilos de texto, fills, efectos, variables enlazadas, propiedades de componentes, render bounds |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | Una decodificación, muchos nodos: una línea JSON por id listado, la salida de `nodes` con su `nodeId`, o `{nodeId, error}` |
| `variables <file> [node]` | Las variables enlazadas de un frame; sin nodo, todas las variables locales |
| `bundle <file> <node> <out.zip> [--states]` | Un bundle de diseño para un frame; imprime `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | Muchos bundles a partir de una sola decodificación, en paralelo |
| `verify <bundle>` | Comprueba un bundle (`.zip` o su directorio extraído) contra su manifiesto: archivos verificados, ausentes y modificados, assets que faltan y las fuentes que hay que confirmar en el proyecto |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP o AVIF; `--max-kb` reduce la calidad con pérdida hasta que quepa |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local de solo lectura; `--out` escribe un snapshot offline independiente |
| `compare <file> <nodes.json> <node>` | Diff contra una exportación REST del mismo frame |
| `node <file> <node>` | Un nodo `.fig` sin procesar |
| `mcp` | Servidor MCP local por stdio |
| `update` | Reemplaza el binario por la última release |

Para variantes de componentes relacionadas, usa `inspect --states` o `bundle --states` (MCP: `states: true`, false por defecto; sin cambiar la configuración). La evidencia conserva marcas de origen, datos desconocidos y truncamiento. Los bundles añaden `design/states.json` y un resumen en `context.md`. Consulta el alcance y los límites en [evidencia de estados](../wiki/states.md).

Compara dos variantes del mismo component set con `inspect <file> <variantA> --compare-variant <variantB>` (MCP `compareVariant`). `variantComparison` muestra diferencias de campos y conjuntos de valores almacenados, con IDs de origen y datos desconocidos, sin emparejar nodos hijos. Consulta los [límites](../wiki/states.md#compare-two-variants).

Un argumento de nodo acepta `12:345`, `12-345` o una URL de Figma con `?node-id=`. `<file>` también puede ser `gs://bucket/object`, descargado con tu login de `gcloud` y guardado en caché. Consulta la [guía de snapshots en GCS](../wiki/gcs.md) para la entrega mediante un objeto fijo, los permisos necesarios y los límites de actualización. Cada llamada a la CLI decodifica el archivo; MCP solo reutiliza el último archivo local sin cambios. Las llamadas remotas vuelven a analizarlo. Ejecuta `figlens --help` o `figlens inspect --help` (también `-h`) para ver el uso completo de la CLI.

`inspect` incluye el ajuste de tamaño del texto, el truncamiento, el máximo de líneas y la decoración registrados. `handoffDiagnostics` informa de semántica guardada pero no emitida, con recuentos, ejemplos y [límites de cobertura](../wiki/semantics.md#known-handoff-gaps). Las salidas protegen el origen y reemplazan el destino tras completar la escritura. MCP limita las líneas de solicitud a 1 MiB y las respuestas completas a 8 MiB; si se supera el límite, devuelve un error explícito. El JSON completo sigue disponible mediante la CLI.
