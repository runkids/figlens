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
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">Empezar</a> · <a href="#demo">Demo</a> · <a href="#why">Por qué</a> · <a href="#bundle">Qué recibe el agente</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limitaciones</a>
</p>

**El problema.** Tu agente necesita valores de layout, texto y recursos para implementar una pantalla. En un servidor remoto o en CI, ese contexto debe estar disponible junto al código sin depender de una aplicación de escritorio en ejecución.

**La solución.** En Figma, **File > Save local copy**. Coloca `design.fig` junto a tu código. figlens lo analiza en local y le da a tu agent layout, texto, estilos, variables, iconos, imágenes y un render de referencia de cualquier frame. Un binario nativo, cero llamadas de red a Figma.

<a id="start"></a>

## Primeros pasos

**¿Aún no tienes un archivo de diseño?** [Comprueba tu entorno con el pequeño ejemplo público](examples/first-success/README.md): verifica el binario elegido, encuentra un frame y valida su bundle. No se necesita un modelo.

Pon `design.fig` en tu proyecto y elige tu agent.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Inicia una sesión nueva y comprueba `/mcp`. Describe la pantalla con tus propias palabras: MCP incluye guías bajo demanda para planificación, implementación y preparación de imágenes; no hace falta instalar el skill por separado.

**Pi** (1.3.0+): un solo paquete incluye la CLI, MCP, el viewer y el skill.

```sh
pi install npm:@runkids/figlens
```

**Cualquier cliente MCP** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

[Instalación independiente](docs/README-es.md#install)

Luego pide una pantalla:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

El agent encuentra el frame, reúne los datos del diseño, los recursos y una imagen de referencia, y construye la pantalla con tu stack. El flujo compartido guía la comprobación de fuentes y recursos; informa de lo que falta mientras continúa la implementación del layout con alternativas documentadas. Pídele los archivos modificados y una comparación en el navegador.

<a id="demo"></a>

## Demo: Pi construye una pantalla React desde un .fig

[![Pi plans from a local .fig, builds a React screen and checks it beside a live browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

El GIF en bucle reproduce el vídeo editado a triple velocidad. Haz clic para ver el vídeo completo de 78 segundos.

**[Video de 78 segundos](assets/taskflow-demo.mp4).** Pi ejecuta `/figlens plan`, lee un frame de dashboard de un `.fig` local, lo construye con React y Tailwind CSS, y luego revisa y corrige la UI en ejecución. Time-lapse editado, no un benchmark ni una promesa de fidelidad pixel-perfect. Diseño: [Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [notas de la grabación](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

**[De .fig a HTML/CSS editable](examples/task-list/README.md):** en un estudio controlado de una lista de tareas, los seis intentos basados en figlens conservaron las 19 apariciones de texto visible como texto DOM real y los cuatro iconos. La extracción por CLI y MCP también pasó con la red de Docker desactivada. El caso incluye configuración, mediciones y comparaciones de imágenes independientes.

<a id="why"></a>

## ¿Por qué figlens en lugar del MCP oficial de Figma?

Usa **figlens** cuando el diseño deba viajar como archivo: agents remotos, CI, trabajo sin conexión, traspaso entre agents, lecturas repetidas sin límite. Usa el **MCP oficial de Figma** cuando el agent deba ver el diseño en vivo sin un paso de exportación.

Leer un `.fig` funciona sin conexión. Instalar y actualizar figlens, las entradas `gs://`, tu proveedor de modelos y las dependencias de tu proyecto sí necesitan red.

[Comparación completa](docs/README-es.md#why)

<a id="bundle"></a>

## Qué recibe el agent

Un frame, un zip. Estructura para construir, recursos para usar, una imagen para comprobar.

[Contenido del bundle y ejemplos](docs/README-es.md#bundle) · [¿Qué tan cerca está de Figma?](docs/README-es.md#accuracy)

## Documentación

- <a id="pi"></a>[Extensión de Pi](docs/README-es.md#pi)
- <a id="viewer"></a>[Explora un diseño sin Figma](docs/README-es.md#viewer)
- <a id="mcp-and-skill"></a>[Servidor MCP y skill para agents](docs/README-es.md#mcp-and-skill)
- <a id="commands"></a>[Referencia de la CLI](docs/README-es.md#commands)

<a id="limitations"></a>

## Limitaciones

- **Snapshots, no sincronización en vivo.** Exporta un nuevo `.fig` cuando cambie el diseño. figlens no puede descargar un diseño desde una URL de Figma.
- **Formato no documentado.** figlens decodifica el esquema incrustado en el archivo; las nuevas exportaciones de Figma pueden requerir una actualización de figlens.
- **Fuentes y texto:** no se incluyen archivos de fuentes; el texto con overrides puede conservar el layout guardado de Figma. Usa fuentes autorizadas del proyecto y compara el resultado.
- **Cobertura:** las [mejoras de renderizado](https://github.com/runkids/figlens/issues/36) y la [semántica de entrega](https://github.com/runkids/figlens/issues/37) se siguen en el gestor de issues. Consulta la [cobertura semántica actual](wiki/semantics.md).
- **Revisa la salida.** Lee los `warnings` del bundle, `missing-assets.json` y las variables sin resolver en `context.md`, y luego compara la implementación con el diseño.
- Las limitaciones visibles conocidas aparecen en `_figlens.renderDiagnostics` de los nodos, los resúmenes limitados de `inspect` y los `warnings` del bundle; `missing-assets.json` solo contiene fallos de recursos. La ausencia de advertencias no garantiza fidelidad de píxeles.

## Ayuda a que figlens crezca

Si figlens convirtió un diseño en código para ti, [dale una estrella al repo](https://github.com/runkids/figlens) para que otros lo encuentren. ¿Encontraste un archivo que no puede leer? [Abre un issue](https://github.com/runkids/figlens/issues) con el comando y lo que esperabas.

El desarrollo ejecuta Rust solo en Docker: `docker compose run --rm dev cargo test`. Consulta [AGENTS.md](AGENTS.md) y el [wiki](wiki/README.md).

## Licencia

[MIT](LICENSE). figlens es un proyecto independiente, sin afiliación, respaldo ni patrocinio de Figma. El formato `.fig` no está documentado y puede cambiar; usa solo archivos y recursos a los que tengas permiso de acceder y compartir.
