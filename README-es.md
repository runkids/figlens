<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Tu diseño de Figma, listo para tu agente de programación.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  Dale a tu agente de programación la estructura, el texto, las variables, los recursos y una imagen de referencia de cualquier pantalla.<br>
  Léelo tantas veces como quieras, en tu portátil, en la nube o en CI.<br>
  <strong>Sin aplicación Figma, inicio de sesión, seat ni límite de la API en la máquina que hace el trabajo.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">Ver la demo</a> · <a href="#quick-start">Probar tu diseño</a> · <a href="#for-ai-agents">Conectar tu agente</a> · <a href="#pi-extension">Usar con Pi</a> · <a href="#how-close-is-it-to-figma">Precisión</a>
</p>

<a id="why-figlens"></a>

## ¿Por qué figlens?

«Construye esta pantalla» debería incluir su estructura, texto y recursos. Con capturas, el agente tiene que estimar espacios y tipografía, y tú sigues exportando iconos a mano. El servidor MCP de Figma lee el diseño real, pero su cuota pertenece al equipo dueño del archivo, un agente puede gastar la cuota del día en un solo prompt y la máquina necesita una sesión de Figma.

figlens lee el diseño desde una copia guardada. En Figma, elige **File > Save local copy** (basta con acceso de lectura, salvo que el propietario haya desactivado la copia). Dale ese `.fig` a tu agente mediante la CLI o el servidor MCP local, y podrá buscar una pantalla, inspeccionar su estructura y texto y extraer los recursos necesarios. O reúne los datos, los recursos y el render de referencia de un frame en un zip que cualquier agente o compañero pueda usar.

| Si te has encontrado con esto | Con figlens puedes |
|---|---|
| «Rate limit exceeded, try again tomorrow», incluso con un seat de pago | Leer el archivo guardado las veces que necesites, sin llamadas a la API de Figma, tokens ni seats |
| Tu agente se ejecuta en la nube o en CI | Leer el diseño con una CLI nativa, sin la aplicación Figma ni sesión de navegador |
| No paras de explicar medidas y exportar recursos | Proporcionar estructura, tipografía, instancias expandidas, variables vinculadas y recursos originales |
| Otro agente o compañero necesita la misma pantalla | Compartir un bundle con datos, recursos, render de referencia e instrucciones |

Los diseñadores siguen trabajando en Figma. Tu agente utiliza el stack del proyecto. figlens proporciona el contexto del diseño; el agente de programación escribe la implementación.

<details>
<summary>El formato .fig no está documentado. ¿Qué pasa cuando Figma lo cambie?</summary>

figlens no adivina el formato. Cada versión se comprueba contra las exportaciones JSON, PNG y SVG del propio Figma de los mismos archivos ([resultados](#how-close-is-it-to-figma)), así que un cambio de formato aparece como una regresión medida, no silenciosa. La copia que ya tienes guardada sigue funcionando con la versión de figlens que la leyó; solo los guardados nuevos desde un Figma cambiado necesitan una actualización de figlens, que `figlens update` descarga.

</details>

<a id="from-a-local-fig-to-working-html"></a>

## De un .fig local a una interfaz HTML funcional

[![Pi construye una interfaz funcional desde un archivo Figma local, con vista previa junto al terminal](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[Ver el vídeo](assets/demo-8x.mp4)** · 45 segundos de cámara rápida, a **8×**. Una sesión real de Pi en Orca: leer el diseño local, crear HTML/CSS y probar el diálogo para compartir y el área de entrada en la vista previa. La interfaz HTML utiliza 19 iconos SVG del bundle.

| Este ejemplo | Resultado medido |
|---|---|
| Archivo `.fig` local | 57.5 MiB |
| Frame seleccionado | 153 nodos, 25 recursos SVG, ninguno ausente |
| Extracción del bundle | **0.47 s de mediana** en cinco ejecuciones con caché caliente |

El tiempo corresponde a `figlens bundle`, **no al tiempo de implementación de la IA**. Medido con figlens 0.1.2 en un contenedor Docker ARM64; consulta los [comandos, resultados individuales y notas de grabación](wiki/accuracy.md#local-design-to-html-demo). Pi escribe el código; figlens aporta los datos. El proveedor de IA sigue usando una conexión de red.

<details>
<summary>Crédito del diseño y limitaciones de la demo</summary>

El frame «List views» procede de [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). La demo utiliza la fuente sans del sistema y Georgia porque las fuentes Anthropic originales no están incluidas. Share abre un diálogo local; las respuestas quedan en la página y se borran al recargar. La grabación usa capturas de la ventana real de Orca, por lo que el movimiento es menos fluido que en una captura de vídeo nativa.

</details>

<a id="quick-start"></a>

## Inicio rápido

<a id="1-open-your-design-locally"></a>

### 1. Abre el diseño localmente

En Figma, elige **File > Save local copy** y guarda el archivo como `design.fig`. Después ejecuta:

```sh
npx @runkids/figlens preview design.fig
```

Abre la URL local que aparece en el terminal. Explora las páginas y selecciona la pantalla que quieres construir. Allí puedes inspeccionar espacios, tipografía, variables y recursos.

Requiere Node.js 18+. La primera ejecución descarga, verifica y almacena en caché un binario nativo. Las [instalaciones independientes](#installation) no necesitan Node.js, Rust ni Docker. Detén el visor con Ctrl+C.

<a id="2-give-the-screen-to-your-agent"></a>

### 2. Entrega la pantalla a tu agente

Descarga el bundle del frame seleccionado desde el visor. Dale el zip a tu agente de programación junto con esta instrucción:

> Extrae este bundle en su propio directorio y lee context.md. Construye esta pantalla con el stack y las convenciones del proyecto. Usa los valores de estructura y los recursos proporcionados y comprueba el resultado con references/screen.png. Informa de recursos ausentes o fuentes no disponibles.

El bundle incluye estructura, recursos e imagen de referencia; el agente puede leerlo con herramientas de archivos habituales. Para que busque e inspeccione pantallas directamente, [conecta la CLI o el servidor MCP](#for-ai-agents). [Los usuarios de Pi pueden pedir una pantalla por su nombre](#pi-extension).

<a id="3-check-the-implementation"></a>

### 3. Comprueba la implementación

Importa una captura de tu implementación en el visor. Compárala junto al diseño o superpón ambas con opacidad ajustable para detectar diferencias. La captura permanece en el navegador. Es una comprobación visual; figlens no calcula una puntuación de precisión para tu implementación.

**El `.fig` es una instantánea.** Guarda una copia local nueva cuando cambie el diseño. Una URL de Figma puede seleccionar un nodo de esa copia; no descarga el diseño. Tras la instalación, la lectura funciona sin conexión; el proveedor de IA y las dependencias del proyecto pueden seguir necesitando la red.

<details>
<summary>¿Prefieres el terminal? Busca un frame y empaquétalo directamente.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Sustituye `12:345` por un ID del árbol o de los resultados de búsqueda. Un nodo puede ser un ID (`12:345`), su formato URL (`12-345`) o una URL de Figma con `?node-id=`. `find` busca nombres de capas o texto; `inspect` muestra hijos de instancias expandidos y un resumen compacto de estructura y texto. Consulta [Comandos](#commands).

</details>

<a id="browse-and-hand-off-a-design"></a>

## Explorar y entregar un diseño

[![Inspección local del diseño MCP Apps con árbol de capas, estructura y propiedades de componentes](assets/preview-inspector.png)](assets/pi-demo.mp4)

El visor local permite explorar un diseño guardado sin sesión de Figma:

- Explora páginas y capas, busca nombres y texto, mide distancias e inspecciona estructura, tipografía, variables, propiedades de componentes y CSS.
- Exporta una capa como SVG, PNG, JPG, WebP o AVIF, o descarga sus imágenes de relleno originales.
- Copia su ID, contexto estructurado o instrucciones de implementación; descarga un bundle de la selección exacta, incluidas subcapas de instancias.

¿Quieres enviar un diseño que otra persona pueda explorar? Exporta una instantánea HTML independiente:

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

Se abre sin servidor ni red e incluye datos, imágenes y un bundle del ámbito de la instantánea. Compártela solo con personas autorizadas a ver esos recursos. Seleccionar un hijo cambia el contexto inspeccionado; el bundle incrustado mantiene el ámbito de la instantánea.

El visor en vivo vigila el archivo local y se actualiza después de guardados correctos; los guardados parciales o fallidos marcan la vista como desactualizada. Las instantáneas HTML permanecen fijas. Consulta la [guía de vista previa](wiki/preview.md) para controles, caché, automatización con agentes de navegador y limitaciones.

<a id="pi-extension"></a>

## Extensión de Pi

**Dale un `.fig` a Pi y pídele la pantalla.** El paquete incluye CLI, MCP, vista previa, extensión y skill para agentes:

```sh
pi install npm:@runkids/figlens
```

Reinicia Pi o ejecuta `/reload` y haz la petición como de costumbre:

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi encuentra la pantalla, lee su estructura y recursos, inspecciona el proyecto e implementa con su stack existente. No necesitas conocer los IDs de frames ni ejecutar `/figlens open`. Si los nombres son ambiguos, Pi puede mostrar algunos candidatos. MCP se inicia automáticamente; no hace falta instalar la CLI globalmente.

![Pide una pantalla a Pi: encuentra el diseño, usa tu stack y comprueba el resultado](assets/pi-workflow.svg)

**Reutilización opcional de componentes:** pide a Pi que guarde correspondencias reutilizables y usa `/figlens review` para aceptar o editar propuestas pendientes del proyecto. Las decisiones quedan en `.figlens/component-mappings.md`; las variantes o el código registrado que cambien requieren revisión. No necesitas correspondencias para construir una pantalla.

[![Inspeccionar el diseño local, revisar un componente HTML existente en Pi y reutilizarlo en el siguiente frame](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Ver el flujo opcional de correspondencias](assets/pi-demo.mp4)** · Grabación editada de 20 segundos de v0.3.0: inspeccionar MCP Apps, aprobar una correspondencia preparada con HTML existente y reutilizarla en una nueva sesión. No se muestra ni se mide la generación del modelo. [Guía, comandos y limitaciones](pi/README.md) · [Notas de grabación y verificación](wiki/pi.md#demo-and-verification).

Requiere Pi 1.3.0+. La extensión npm utiliza su lanzador CLI incluido; el primer inicio, incluido el arranque automático de MCP, descarga y verifica el binario nativo correspondiente. Homebrew y los archivos de release proporcionan la CLI nativa y el visor. Para desarrollo, consulta la [configuración desde el código fuente](pi/README.md#install-once).

<a id="the-design-bundle"></a>

## El bundle de diseño

Un frame, un zip. Estructura para construir, recursos para usar y una imagen con la que comparar.

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

¿Solo necesitas una parte? Exporta JSON o renderiza un nodo directamente:

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

La exportación SVG admite frames, grupos, iconos y subcapas exactas de instancias: cada subárbol seleccionado genera un SVG independiente con imágenes originales incrustadas y texto como contornos de glifos. No necesitas extraer un bundle. SVG conserva las dimensiones originales; `--scale` y `--quality` solo se aplican a formatos ráster. A través de MCP, las exportaciones ráster se reducen para caber en `maxSize` (1600 px en el lado más largo por defecto; 0 lo desactiva), así las imágenes inline se mantienen pequeñas.

<a id="how-close-is-it-to-figma"></a>

## ¿Cuánto se parece a Figma?

figlens se verifica con las exportaciones JSON, PNG y SVG de Figma. Los resultados registrados en **826 frames de producción** y **cinco archivos Community** incluyen:

| Comprobación | Resultado |
|---|---|
| Estructura, nombres, visibilidad, límites y texto de los nodos | Idénticos en 771 / 826 frames de producción |
| Imágenes de relleno originales | 2,608 / 2,608 idénticas byte a byte |
| Recursos SVG | 5,014 / 5,197 dentro del umbral de diferencia de píxeles |
| Bundles de frames Community | 306 / 306 sin recursos ausentes |

Son mediciones de esos archivos, no una garantía para cualquier diseño. Consulta el [informe de precisión](wiki/accuracy.md) para fechas, umbrales, resultados de renderizado y método.

**Limitaciones conocidas:** aún no se emiten constraints, layoutAlign/layoutGrow, cuadrículas ni definiciones de propiedades de componentes. El renderizado omite el desenfoque de fondo y algunos ajustes de imagen (contraste, temperatura, matiz, luces y sombras), y dibuja los degradados angulares y de diamante como radiales. El texto usa contornos de glifos guardados, por lo que el renderizado no necesita fuentes, pero el texto sobrescrito puede conservar la última distribución de Figma. Los archivos de fuentes para tu implementación no están incluidos.

<a id="for-ai-agents"></a>

## Para agentes de IA

Usa la CLI mediante un skill para agentes o conecta el mismo binario como servidor MCP.

<a id="agent-skill"></a>

### Skill para agentes

El [skill de figlens](skills/figlens/SKILL.md) enseña al agente a encontrar frames, leer el bundle por partes y construir usando sus valores.

Instálalo con la [Skills CLI](https://github.com/vercel-labs/skills):

```sh
npx skills add runkids/figlens --skill figlens
```

Para gestionar y actualizar skills en Claude Code, Codex, Cursor, Gemini CLI y otras herramientas, usa [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` los mantiene actualizados. También puedes copiar `skills/figlens/` al directorio de skills del agente (`~/.claude/skills/` para Claude Code). Los archivos de release también lo incluyen.

<a id="mcp-server"></a>

### Servidor MCP

Gestiona servidores MCP y skills con [skillshare](https://github.com/runkids/skillshare). Define figlens una vez y sincroniza su conexión con Claude Code, Codex y Cursor:

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

Consulta la [guía MCP de Skillshare](https://skillshare.runkids.cc/docs/reference/commands/mcp/) para otros clientes e importación de configuraciones existentes.

Configuración directa de Claude Code:

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

Otros clientes MCP:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

Con una instalación independiente de `figlens`, usa:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Para limitar el servidor a ciertas carpetas, añade `--root`: `"args": ["mcp", "--root", "/ruta/a/disenos"]`. Cada `file` y ruta de salida que nombre una llamada debe estar bajo una de esas raíces; repite `--root` para más carpetas. Las llamadas cuyos argumentos se salen del esquema de la herramienta también se rechazan, ya que las conversiones heredadas podrían convertir cualquiera de ellos en una ruta.

El servidor se ejecuta localmente por stdio y expone `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` y `export`. Usa rutas absolutas: las relativas se resuelven desde el directorio donde el cliente inició figlens.

Empieza con `find` e `inspect` para seleccionar un frame. `export` guarda el archivo y devuelve metadatos. Los formatos ráster también devuelven contenido de imagen MCP, por lo que el agente puede ver el diseño sin abrir un archivo local; SVG solo devuelve metadatos. PNG tiene la compatibilidad más amplia; reduce `scale` para frames grandes. `find`, `inspect`, `bundle` y `export` también devuelven JSON estructurado.

<a id="installation"></a>

## Instalación

Usa `npx @runkids/figlens` con Node.js 18+ o instala la CLI independiente:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Hay binarios para macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) y Windows (x86_64; ARM64 mediante emulación). Las instalaciones independientes se actualizan con `figlens update`.

<a id="commands"></a>

## Comandos

| Comando | Salida |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | IDs, tipos y nombres de nodos; texto indentado (menos contexto para el agente) o JSON anidado con `--json` |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Búsqueda de subcadenas en nombres y texto sin distinguir mayúsculas; rutas, tamaños y vistas previas. 20 resultados por defecto (1–200); `truncated` indica más |
| `inspect <file> <node> [--depth N]` | Resumen compacto de estructura/texto con instancias expandidas. Profundidad 1 por defecto (0–10), hasta 200 nodos; usa `nodes` para datos completos |
| `nodes <file> <node>` | JSON con estructura REST: instancias y sobrescrituras expandidas, auto-layout, estilos de texto, rellenos, efectos, variables vinculadas, propiedades y límites de renderizado |
| `variables <file> [node]` | Variables vinculadas a un frame; sin nodo, todas las variables locales |
| `bundle <file> <node> <out.zip>` | Bundle de diseño de un frame; devuelve `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Múltiples bundles en paralelo con una sola decodificación; una línea JSON por frame, como arriba o `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP o AVIF; `--max-kb` reduce la calidad con pérdida hasta ajustarse; PNG/SVG fallan si exceden el límite; devuelve `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Visor local de solo lectura; `--out` exige un nodo y crea una instantánea independiente sin conexión |
| `compare <file> <nodes.json> <node>` | Diferencias con una exportación REST del mismo frame |
| `node <file> <node>` | Un nodo `.fig` sin procesar |
| `mcp` | Servidor MCP local por stdio |
| `update` | Sustituye el binario por la última release |

`find` busca capas guardadas y devuelve IDs, rutas de ancestros, tamaños locales y vistas previas del texto. `inspect` expande instancias y limita la salida a 200 nodos y 200 caracteres por texto; `truncated` y `textTruncated` indican omisiones. Cada llamada de la CLI decodifica el archivo; el servidor MCP conserva el archivo analizado entre llamadas hasta que cambia en disco.

`<file>` también admite `gs://bucket/object`, descargado con tu sesión de `gcloud` y almacenado en caché. Ejecuta `figlens --help` para ver el uso.

<a id="help-figlens-grow"></a>

## Ayuda a que figlens crezca

Si figlens te ayuda a convertir tu próximo diseño en código, [dale una estrella](https://github.com/runkids/figlens) para que otros lo encuentren. ¿Un diseño causa problemas? [Abre una issue](https://github.com/runkids/figlens/issues) con el comando ejecutado y el resultado esperado.

Durante el desarrollo, Rust solo se ejecuta en Docker: `docker compose run --rm dev cargo test`. `make devc` abre un shell donde `figlens` ejecuta el código de este checkout; `make help` muestra los objetivos del contenedor. Reopen in Container de VS Code usa el mismo servicio. Consulta [AGENTS.md](AGENTS.md) para las reglas y el [wiki](wiki/README.md) para temas técnicos.

<a id="license"></a>

## Licencia

[MIT](LICENSE). figlens es un proyecto independiente, sin afiliación, respaldo ni patrocinio de Figma. El formato `.fig` no está documentado y puede cambiar; figlens lee archivos guardados desde tu propia cuenta de Figma.
