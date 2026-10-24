<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Dein Figma-Design, bereit für deinen Coding-Agenten.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  Gib deinem Coding-Agenten Layout, Text, Variablen, Assets und ein Referenzbild jedes Bildschirms.<br>
  Lies es so oft du willst, auf dem Laptop, in der Cloud oder in CI.<br>
  <strong>Ohne Figma-App, Anmeldung, Seat oder API-Limit auf dem ausführenden Rechner.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://www.npmjs.com/package/@runkids/figlens"><img src="https://img.shields.io/npm/dm/@runkids/figlens?label=npm%20downloads&amp;color=8b78c8" alt="npm downloads per month"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#installation">Installieren</a> · <a href="#from-a-local-fig-to-working-html">Demo ansehen</a> · <a href="#quick-start">Eigenes Design testen</a> · <a href="#for-ai-agents">Agent verbinden</a> · <a href="#pi-extension">Mit Pi verwenden</a> · <a href="#how-close-is-it-to-figma">Genauigkeit</a>
</p>

## Installation

Nutze `npx @runkids/figlens` mit Node.js 18+ oder installiere die eigenständige CLI:

| Plattform | Befehl |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Release-Binärdateien gibt es für macOS (Apple silicon, Intel), Linux (arm64, x86_64, statisches musl) und Windows (x86_64; ARM64 per Emulation). Eigenständige Installationen werden mit `figlens update` aktualisiert.

<a id="quick-start"></a>

## Schnellstart

<a id="1-open-your-design-locally"></a>

### 1. Design lokal öffnen

Wähle in Figma **File > Save local copy** und speichere die Datei als `design.fig`. Führe dann aus:

```sh
npx @runkids/figlens preview design.fig
```

Öffne die lokale URL aus dem Terminal. Durchsuche die Seiten und wähle den Bildschirm aus, den du umsetzen möchtest. Abstände, Typografie, Variablen und Assets kannst du direkt dort untersuchen.

Erfordert Node.js 18+. Beim ersten Aufruf wird das native Programm heruntergeladen, überprüft und zwischengespeichert. [Eigenständige Installationen](#installation) benötigen weder Node.js noch Rust oder Docker. Beende die Vorschau mit Ctrl+C.

<a id="2-give-the-screen-to-your-agent"></a>

### 2. Bildschirm an den Agenten übergeben

Lade das Bundle des ausgewählten Frames aus der Vorschau herunter. Gib deinem Coding-Agenten die ZIP-Datei und diesen Prompt:

> Entpacke dieses Bundle in ein eigenes Verzeichnis und lies context.md. Implementiere diesen Bildschirm mit dem Stack und den Konventionen dieses Projekts. Verwende die mitgelieferten Layoutwerte und Assets und prüfe das Ergebnis anhand von references/screen.png. Melde fehlende Assets oder nicht verfügbare Schriftarten.

Das Bundle enthält Struktur, Assets und Referenzbild zusammen. Der Agent kann es mit gewöhnlichen Dateiwerkzeugen lesen. Soll er Bildschirme selbst suchen und untersuchen, [verbinde CLI oder MCP-Server](#for-ai-agents). [Pi-Nutzer können Bildschirme beim Namen anfordern](#pi-extension).

<a id="3-check-the-implementation"></a>

### 3. Implementierung prüfen

Importiere einen Screenshot deiner Implementierung in die Vorschau. Vergleiche ihn neben dem Design oder als Überlagerung mit einstellbarer Deckkraft. Der Screenshot bleibt im Browser. Das ist eine visuelle Prüfung; figlens berechnet keinen Genauigkeitswert für deine Implementierung.

**Die `.fig` ist eine Momentaufnahme.** Speichere eine neue lokale Kopie, wenn sich das Design ändert. Eine Figma-URL kann einen Knoten in dieser Kopie auswählen, lädt aber kein Design herunter. Nach der Installation funktioniert das Lesen offline; dein KI-Anbieter und die Projektabhängigkeiten können weiterhin Netzwerkzugriff benötigen.

<details>
<summary>Lieber im Terminal? Frame suchen und direkt bündeln.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Ersetze `12:345` durch eine ID aus dem Baum oder den Suchergebnissen. Ein Knoten kann als ID (`12:345`), URL-Form (`12-345`) oder Figma-URL mit `?node-id=` angegeben werden. `find` durchsucht Ebenennamen oder Text; `inspect` zeigt expandierte Instanzkinder und eine kompakte Layout-/Textübersicht. Details unter [Befehle](#commands).

</details>

<a id="why-figlens"></a>

## Warum figlens?

Zu „Baue diesen Bildschirm“ gehören Layout, Text und Assets. Mit Screenshots muss der Agent Abstände und Typografie schätzen, und Icons exportierst du weiterhin von Hand. Der Figma-MCP-Server liest das echte Design, aber sein Kontingent gehört dem Team, dem die Datei gehört, ein Agent kann das Tageskontingent mit einem einzigen Prompt aufbrauchen, und der Rechner braucht eine Figma-Anmeldung.

figlens liest das Design stattdessen aus einer gespeicherten Kopie. Wähle in Figma **File > Save local copy** (Lesezugriff genügt, sofern der Besitzer das Kopieren nicht deaktiviert hat). Gib die `.fig` deinem Agenten über die CLI oder den lokalen MCP-Server; er kann Bildschirme finden, Layout und Text untersuchen und benötigte Assets extrahieren. Oder bündle Designdaten, Assets und Referenzrender eines Frames in einer ZIP-Datei, die jeder Agent und jeder Kollege nutzen kann.

| Wenn du dieses Problem kennst | Mit figlens kannst du |
|---|---|
| „Rate limit exceeded, try again tomorrow“, selbst mit bezahltem Seat | Die gespeicherte Datei so oft lesen wie nötig, ohne Figma-API-Aufrufe, Tokens oder Seats |
| Dein Agent läuft auf einem Cloud-Server oder in CI | Das Design mit einer nativen CLI lesen, ohne Figma-App oder Browsersitzung |
| Du erklärst ständig Maße und exportierst Assets | Strukturiertes Layout, Typografie, expandierte Instanzen, gebundene Variablen und Original-Assets bereitstellen |
| Ein anderer Agent oder Teamkollege braucht denselben Bildschirm | Ein Frame-Bundle mit Designdaten, Assets, Referenzrender und Anweisungen teilen |

Designer arbeiten weiter in Figma. Dein Agent nutzt den Stack deines Projekts. figlens liefert den Designkontext; der Coding-Agent schreibt die Implementierung.

<details>
<summary>Das .fig-Format ist nicht dokumentiert. Was passiert, wenn Figma es ändert?</summary>

figlens rät nicht beim Format. Jede Version wird gegen Figmas eigene JSON-, PNG- und SVG-Exporte derselben Dateien geprüft ([Ergebnisse](#how-close-is-it-to-figma)), sodass eine Formatänderung als gemessene Regression auffällt, nicht still. Eine bereits gespeicherte Kopie funktioniert weiter mit der figlens-Version, die sie gelesen hat; nur neue Speicherungen aus einem geänderten Figma brauchen ein figlens-Update, das `figlens update` holt.

</details>

<a id="from-a-local-fig-to-working-html"></a>

## Von einer lokalen .fig zu funktionierendem HTML

[![Pi erstellt aus einer lokalen Figma-Datei eine bedienbare Oberfläche mit Live-Vorschau neben dem Terminal](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[Video ansehen](assets/demo-8x.mp4)** · 45 Sekunden Zeitraffer, **8× beschleunigt**. Eine echte Pi-Sitzung in Orca: lokales Design lesen, HTML/CSS erstellen und anschließend Teilen-Dialog und Eingabebereich in der Live-Vorschau ausprobieren. Die HTML-Oberfläche verwendet 19 SVG-Icons aus dem Bundle.

| Dieses Beispiel | Messergebnis |
|---|---|
| Lokale `.fig`-Datei | 57.5 MiB |
| Ausgewählter Frame | 153 Knoten, 25 SVG-Assets, keine fehlenden Assets |
| Design-Bundle extrahieren | **0.47 s Median** aus fünf Durchläufen mit warmem Cache |

Gemessen wird `figlens bundle`, **nicht die Implementierungszeit der KI**. Messung mit figlens 0.1.2 in einem ARM64-Docker-Container; siehe [Befehle, Einzelergebnisse und Aufnahmenotizen](wiki/accuracy.md#local-design-to-html-demo). Pi schreibt den Code; figlens liefert Designdaten. Der KI-Anbieter benötigt weiterhin eine Netzwerkverbindung.

<details>
<summary>Designquelle und Einschränkungen der Demo</summary>

Der Frame „List views“ stammt aus [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). Die Demo nutzt System-Sans und Georgia, weil die originalen Anthropic-Schriften nicht enthalten sind. Share öffnet einen lokalen Dialog; Antworten bleiben auf der Seite und werden beim Neuladen gelöscht. Die Aufnahme besteht aus Screenshots des echten Orca-Fensters, daher sind Bewegungen weniger flüssig als bei einer nativen Videoaufnahme.

</details>

<a id="browse-and-hand-off-a-design"></a>

## Design durchsuchen und übergeben

[![MCP-Apps-Design lokal mit Ebenenbaum, Layout und Komponenteneigenschaften untersuchen](assets/preview-inspector.png)](assets/pi-demo.mp4)

Mit der lokalen Vorschau erkundest du gespeicherte Designs ohne Figma-Sitzung:

- Seiten und Ebenen durchsuchen, Namen und Text suchen, Abstände messen sowie Layout, Typografie, Variablen, Komponenteneigenschaften und CSS untersuchen.
- Eine Ebene als SVG, PNG, JPG, WebP oder AVIF exportieren oder ihre originalen Füllbilder herunterladen.
- Ebenen-ID, strukturierten Kontext oder Implementierungs-Prompt kopieren; ein Bundle für die exakte Auswahl einschließlich Instanz-Unterebenen herunterladen.

Soll jemand anderes das Design durchsuchen können? Exportiere eine eigenständige HTML-Momentaufnahme:

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

Sie öffnet sich ohne Server oder Netzwerk und enthält Designdaten, Bilder und ein Bundle für ihren gesamten Umfang. Teile sie nur mit Personen, die diese Assets sehen dürfen. Die Auswahl eines Kindknotens ändert den untersuchten Kontext; das eingebettete Bundle umfasst weiterhin die ursprüngliche Momentaufnahme.

Die Live-Vorschau überwacht die lokale Datei und aktualisiert sich nach erfolgreichem Speichern. Teilweise oder fehlgeschlagene Speichervorgänge markieren die Ansicht als veraltet. HTML-Momentaufnahmen bleiben unverändert. Bedienelemente, Cache, Browser-Agent-Automatisierung und Grenzen stehen im [Vorschauleitfaden](wiki/preview.md).

<a id="pi-extension"></a>

## Pi-Erweiterung

**Gib Pi eine `.fig` und frage nach dem Bildschirm.** Das [Paket](https://pi.dev/packages/@runkids/figlens) enthält CLI, MCP, Vorschau, Erweiterung und Agent-Skill:

```sh
pi install npm:@runkids/figlens
```

Starte Pi neu oder führe `/reload` aus und formuliere dann deine Anfrage wie gewohnt:

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi findet den Bildschirm, liest Layout und Assets, untersucht dein Projekt und implementiert mit dessen vorhandenem Stack. Du brauchst keine Frame-IDs und musst `/figlens open` nicht ausführen. Bei mehrdeutigen Namen kann Pi einige Kandidaten zeigen. MCP startet automatisch; eine globale CLI-Installation ist nicht erforderlich.

![Pi findet den gewünschten Bildschirm, implementiert mit deinem Stack und prüft das Ergebnis](assets/pi-workflow.svg)

**Optionale Komponentenwiederverwendung:** Bitte Pi, wiederverwendbare Zuordnungen zu speichern. Mit `/figlens review` kannst du ausstehende Vorschläge im Projekt akzeptieren oder bearbeiten. Entscheidungen stehen in `.figlens/component-mappings.md`; geänderte Varianten oder gespeicherter Code müssen geprüft werden. Zum Implementieren eines Bildschirms sind Zuordnungen nicht erforderlich.

[![Lokales Design untersuchen, vorhandene HTML-Komponente in Pi prüfen und im nächsten Frame wiederverwenden](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Optionalen Zuordnungsablauf ansehen](assets/pi-demo.mp4)** · 20 Sekunden bearbeitete Aufnahme aus v0.3.0: MCP-Apps-Design untersuchen, vorbereitete Zuordnung zu vorhandenem HTML bestätigen und in neuer Sitzung wiederverwenden. Die Modellgenerierung wird weder gezeigt noch gemessen. [Anleitung, Befehle und Grenzen](pi/README.md) · [Aufnahme- und Prüfnotizen](wiki/pi.md#demo-and-verification).

Erfordert Pi 1.3.0+. Die npm-Erweiterung verwendet ihren mitgelieferten CLI-Launcher. Beim ersten Start einschließlich automatischem MCP-Start wird das passende native Programm heruntergeladen und überprüft. Homebrew und Release-Archive enthalten native CLI und Vorschau. Für die Entwicklung siehe [Einrichtung aus dem Quellcode](pi/README.md#install-once).

<a id="the-design-bundle"></a>

## Das Design-Bundle

Ein Frame, eine ZIP-Datei. Struktur zum Implementieren, Assets zum Verwenden und ein Bild zum Vergleichen.

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

Nur einen Teil benötigt? JSON exportieren oder einen Knoten direkt rendern:

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

SVG-Exporte unterstützen Frames, Gruppen, Icons und exakte Instanz-Unterebenen. Ein ausgewählter Teilbaum wird zu einem eigenständigen SVG mit eingebetteten Originalbildern und Text als Glyphenumrissen. Kein Entpacken eines Bundles nötig. SVG behält seine Abmessungen; `--scale` und `--quality` gelten nur für Rasterformate. Über MCP werden Rasterexporte auf `maxSize` verkleinert (standardmäßig 1600 px an der längsten Seite; 0 schaltet es ab), damit Inline-Bilder klein bleiben.

<a id="how-close-is-it-to-figma"></a>

## Wie nah ist das Ergebnis an Figma?

figlens wird gegen Figma-eigene JSON-, PNG- und SVG-Exporte geprüft. Die aufgezeichneten Ergebnisse für **826 Produktionsframes** und **fünf Community-Dateien** enthalten:

| Prüfung | Ergebnis |
|---|---|
| Knotenstruktur, Namen, Sichtbarkeit, Grenzen und Text | Identisch in 771 / 826 Produktionsframes |
| Originale Füllbilder | 2,608 / 2,608 byteidentisch |
| SVG-Assets | 5,014 / 5,197 innerhalb des Pixeldifferenz-Schwellwerts |
| Community-Frame-Bundles | 306 / 306 ohne fehlende Assets |

Das sind Messergebnisse für diese Dateien, keine Garantie für jedes Design. Daten, Schwellwerte, Renderergebnisse und Methode stehen im [Genauigkeitsbericht](wiki/accuracy.md).

**Bekannte Grenzen:** constraints, layoutAlign/layoutGrow, Layout-Raster und Definitionen von Komponenteneigenschaften werden noch nicht ausgegeben. Der Renderer überspringt Hintergrundunschärfe und manche Bildkorrekturen (Kontrast, Temperatur, Farbton, Lichter, Schatten) und zeichnet Winkel- und Rautenverläufe als radiale Verläufe. Text nutzt gespeicherte Glyphenumrisse und benötigt deshalb beim Rendern keine Schriftdateien. Überschriebener Text kann jedoch Figmas letztes Layout behalten. Schriftdateien für deine Implementierung sind nicht enthalten.

<a id="for-ai-agents"></a>

## Für KI-Agenten

Verwende die CLI über einen Agent-Skill oder verbinde dasselbe Programm als MCP-Server.

### Agent-Skill

Der [figlens-Skill](skills/figlens/SKILL.md) zeigt deinem Agenten, wie er Frames findet, das Bundle in kleinen Teilen liest und anhand seiner Werte implementiert.

Für eine genaue Umsetzung verbindet der [Planungsablauf](skills/figlens/references/planning.md) den gewünschten Umfang mit vorhandenem Code, Bildauslieferung und Abnahmekriterien. In Pi erstellt `/figlens plan <request>` vor der Umsetzung einen Plan; ein kleiner Bereich bleibt eine kleine Aufgabe. Mit `/figlens open [description]` findest du ein Design anhand dessen, was du bereits weißt. Pi fragt nach fehlenden Angaben, ohne Node-IDs vorauszusetzen. Mit `/figlens preview [description]` startest du den lokalen interaktiven Viewer und erhältst seinen Link. Pi findet Datei und Bildschirm, ohne Node-IDs vorauszusetzen.

Mit der [Skills CLI](https://github.com/vercel-labs/skills) installieren:

```sh
npx skills add runkids/figlens --skill figlens
```

Zum Verwalten und Aktualisieren von Skills in Claude Code, Codex, Cursor, Gemini CLI und weiteren Tools nutze [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` hält sie aktuell. Du kannst auch `skills/figlens/` in das Skills-Verzeichnis des Agenten kopieren (für Claude Code `~/.claude/skills/`). Release-Archive enthalten den Skill ebenfalls.

### MCP-Server

Verwalte MCP-Server und Agent-Skills gemeinsam mit [skillshare](https://github.com/runkids/skillshare). Definiere figlens einmal und synchronisiere die Verbindung mit Claude Code, Codex und Cursor:

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

Weitere Clients und der Import bestehender Einstellungen stehen im [Skillshare-MCP-Leitfaden](https://skillshare.runkids.cc/docs/reference/commands/mcp/).

Für eine direkte Claude-Code-Einrichtung:

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

Andere MCP-Clients:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

Mit einer eigenständigen `figlens`-Installation:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Um den Server auf bestimmte Ordner zu beschränken, fügen Sie `--root` hinzu: `"args": ["mcp", "--root", "/pfad/zu/designs"]`. Jede `file`- und Ausgabedatei eines Tool-Aufrufs muss dann unter einem der Wurzelordner liegen; wiederholen Sie `--root` für weitere Ordner. Aufrufe, deren Argumente vom Tool-Schema abweichen, werden ebenfalls abgelehnt, da die alten Argumentumwandlungen jedes davon zu einem Pfad machen könnten.

Der Server läuft lokal über stdio und stellt `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` und `export` bereit. Verwende absolute Pfade: relative Pfade beziehen sich auf das Verzeichnis, aus dem der Client figlens gestartet hat.

Wähle mit `find` und `inspect` einen Frame. `export` speichert die Datei und liefert Metadaten. Rasterformate liefern zusätzlich MCP-Bildinhalte, damit der Agent das Design ohne Öffnen einer lokalen Datei sehen kann. SVG liefert nur Metadaten. PNG hat die breiteste Client-Unterstützung; reduziere `scale` bei großen Frames. `find`, `inspect`, `bundle` und `export` liefern auch strukturiertes JSON.

<a id="commands"></a>

## Befehle

| Befehl | Ausgabe |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Knoten-IDs, Typen und Namen; eingerückter Text (weniger Agent-Kontext) oder mit `--json` verschachteltes JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Teilstringsuche in Ebenennamen und Text ohne Beachtung der Groß-/Kleinschreibung; Pfade, Größen, Vorschauen. Standard 20 Ergebnisse (1–200); `truncated` zeigt weitere an |
| `inspect <file> <node> [--depth N] [--images]` | Kompakte expandierte Layout-/Textübersicht. Standardtiefe 1 (0–10), höchstens 200 Knoten; vollständige Daten mit `nodes`. `--images` ergänzt Quellpixel, Bytes und Zuschnittbereiche je Verwendung; Pixelzahlen erklären keine Unschärfeursache. |
| `nodes <file> <node>` | REST-förmiges JSON: expandierte Instanzen und Überschreibungen, Auto-Layout, Textstile, Füllungen, Effekte, gebundene Variablen, Komponenteneigenschaften und Rendergrenzen |
| `variables <file> [node]` | Gebundene Variablen eines Frames; ohne Knoten alle lokalen Variablen |
| `bundle <file> <node> <out.zip>` | Design-Bundle für einen Frame; liefert `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Mehrere Bundles parallel aus einer Decodierung; eine JSON-Zeile pro Frame wie oben oder `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP oder AVIF; `--max-kb` senkt verlustbehaftete Qualität bis zur Grenze; PNG/SVG scheitern bei Überschreitung; liefert `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Lokale schreibgeschützte Vorschau; `--out` benötigt einen Knoten und erstellt eine eigenständige Offline-Momentaufnahme |
| `compare <file> <nodes.json> <node>` | Differenzen zum REST-Export desselben Frames |
| `node <file> <node>` | Ein unbearbeiteter `.fig`-Knoten |
| `mcp` | Lokaler MCP-Server über stdio |
| `update` | Ersetzt das Programm durch die neueste Release |

`find` durchsucht gespeicherte Ebenen und liefert IDs, Vorfahrenpfade, lokale Größen und Textvorschauen. `inspect` expandiert Instanzen und begrenzt die Ausgabe auf 200 Knoten und 200 Zeichen pro Text. `truncated` und `textTruncated` markieren Auslassungen. Jeder CLI-Aufruf decodiert die Datei; der MCP-Server behält die geparste Datei über Aufrufe hinweg, bis sie sich auf der Festplatte ändert.

`<file>` kann auch `gs://bucket/object` sein, das mit deiner `gcloud`-Anmeldung heruntergeladen und zwischengespeichert wird. Hilfe: `figlens --help`.

<a id="help-figlens-grow"></a>

## figlens unterstützen

Wenn figlens dir hilft, dein nächstes Design in Code umzusetzen, [gib dem Projekt einen Stern](https://github.com/runkids/figlens), damit andere es finden. Ein Design bereitet Probleme? [Erstelle ein Issue](https://github.com/runkids/figlens/issues) mit dem ausgeführten Befehl und dem erwarteten Ergebnis.

Bei der Entwicklung läuft Rust nur in Docker: `docker compose run --rm dev cargo test`. `make devc` öffnet eine Shell, in der `figlens` den Quellcode dieses Checkouts ausführt; `make help` listet Container-Ziele auf. VS Codes Reopen in Container verwendet denselben Dienst. Projektregeln stehen in [AGENTS.md](AGENTS.md), technische Themen im [Wiki](wiki/README.md).

<a id="license"></a>

## Lizenz

[MIT](LICENSE). figlens ist ein unabhängiges Projekt, ohne Zugehörigkeit zu, Unterstützung durch oder Sponsoring von Figma. Das `.fig`-Format ist undokumentiert und kann sich ändern. figlens liest Dateien, die du aus deinem eigenen Figma-Konto gespeichert hast.
