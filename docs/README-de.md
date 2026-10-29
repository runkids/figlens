# figlens — Benutzerhandbuch

[← README](../README-de.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [Eigenständige Installation](#install)
- [Warum figlens statt des offiziellen Figma MCP?](#why)
- [Was der Agent bekommt](#bundle)
- [Pi-Extension](#pi)
- [Ein Design ohne Figma durchsuchen](#viewer)
- [MCP-Server und Agent-Skill](#mcp-and-skill)
- [CLI-Referenz](#commands)

<a id="install"></a>

## Eigenständige Installation

**Kein Node.js?** Installiere das eigenständige Binary und verwende stattdessen `"command": "figlens", "args": ["mcp"]`:

| Plattform | Befehl |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

<a id="why"></a>

## Warum figlens statt des offiziellen Figma MCP?

Nutze **figlens**, wenn das Design als Datei wandern soll: Remote-Agents, CI, Offline-Arbeit, Übergabe zwischen Agents, unbegrenzt wiederholte Lesezugriffe. Nutze den **offiziellen Figma MCP**, wenn der Agent das Live-Design ohne Export-Schritt sehen muss.

| | figlens | Offizieller Figma MCP |
|---|---|---|
| Quelle | Exportierter `.fig`-Snapshot | Live-Figma-Dateien |
| Voraussetzung | Die Datei | Figma-Anmeldung; der Desktop-Server braucht die App |
| Rate Limits | Keine, lokales Parsen | Limits des Figma-Dienstes |
| Remote-Agent / CI | Datei kopieren, Binary ausführen | Remote-Server mit Anmeldung |
| Design-Änderungen | Neue `.fig` exportieren | Automatisch |
| Übergabe | `.fig` oder Frame-Bundle teilen | Figma-Zugriff teilen |
| Genauigkeit | Unabhängiger Parser mit [bekannten Lücken](../README-de.md#limitations) | Von Figma gepflegt |

Eine `.fig` zu lesen funktioniert offline. Installieren und Aktualisieren von figlens, `gs://`-Eingaben, dein Modellanbieter und die Abhängigkeiten deines Projekts brauchen weiterhin ein Netzwerk.

<a id="bundle"></a>

## Was der Agent bekommt

Ein Frame, eine Zip. Struktur zum Bauen, Assets zum Verwenden, ein Bild zum Abgleichen.

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

Brauchst du nur ein Teil? Die CLI erledigt das direkt:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # Frame-IDs nach Name oder Text
npx -y @runkids/figlens inspect design.fig 12:345                 # kompakte Layout- und Textzusammenfassung
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON im REST-Format
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP oder AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # die Zip von oben
```

SVG-Exporte betten originale Bildfüllungen ein und wandeln Text in Glyphen-Umrisse um, daher werden keine Schriften benötigt. Über MCP kommen Raster-Exporte außerdem als Inline-Bilder zurück, sodass der Agent das Design sieht, ohne eine Datei zu öffnen.

<a id="accuracy"></a>

### Wie nah ist es an Figma?

Gemessen an Figmas eigenen JSON-, PNG- und SVG-Exporten für **826 Produktions-Frames** und fünf Community-Dateien (2026-10-22):

| Prüfung | Ergebnis |
|---|---|
| Knotenstruktur, Namen, Sichtbarkeit, Bounds und Text | Identisch in 771 / 826 Frames |
| Originale Bildfüllungen | 2.608 / 2.608 byte-identisch |
| SVG-Assets | 5.014 / 5.197 innerhalb des Pixelabweichungs-Schwellenwerts |
| Community-Frame-Bundles | 306 / 306 ohne fehlende Assets |

Ergebnisse für diese Dateien, keine Garantie für jedes Design. Methode und Schwellenwerte: [Genauigkeitsbericht](../wiki/accuracy.md).

<a id="pi"></a>

## Pi-Extension

```sh
pi install npm:@runkids/figlens
```

Installationsanleitungen: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

Starte Pi neu oder führe `/reload` aus und bitte dann in normalen Worten um einen Bildschirm. Pi findet den Frame, liest ihn über den automatisch registrierten MCP-Server und baut mit deinem Stack. Keine Node-IDs, keine globale CLI.

- `/figlens plan <request>` erstellt nur einen Plan und ändert die Anwendung nicht.
- `/figlens yolo <request>` plant, baut, prüft und räumt den eigenen temporären Plan auf.
- `/figlens preview [description]` öffnet den lokalen Viewer.
- `/figlens review` genehmigt wiederverwendbare Komponenten-Zuordnungen, die der Agent vorgeschlagen hat.

![Bitte Pi um einen Bildschirm, lass es das Design finden, mit deinem Stack bauen und das Ergebnis prüfen](../assets/pi-workflow.svg)

Vollständige Anleitung, Befehle und Grenzen: [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Ein Design ohne Figma durchsuchen

[![Das MCP-Apps-Design lokal inspizieren, mit Layer-Baum, Layout und Komponenten-Eigenschaften](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # lokaler Viewer, Ctrl+C zum Beenden
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # Offline-HTML-Snapshot zum Weitergeben an jeden
```

Durchsuche Seiten und Layer, suche, miss, inspiziere Layout, Typografie, Variablen und CSS, exportiere jeden Layer, kopiere einen Implementierungs-Prompt oder lade ein Bundle für die Auswahl herunter. Lege einen Screenshot deines Builds über das Design, um beide zu überlagern. Details: [Preview-Anleitung](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## MCP-Server und Agent-Skill

**MCP.** Der Server läuft lokal über stdio und stellt `workflow`, `tree`, `find`, `search`, `inspect`, `nodes`, `variables`, `bundle`, `verify` und `export` bereit. Übergib absolute Pfade. Mit `--root /path/to/designs` (wiederholbar) beschränkst du jeden Datei- und Ausgabepfad auf ausgewählte Ordner. Raster-Exporte werden auf `maxSize` (Standard 1600 px) verkleinert, damit Inline-Bilder klein bleiben; SVG liefert nur Metadaten.

Über `workflow` kann der Agent die integrierten Anleitungen `build`, `audit`, `images`, `plan`, `planning-details` oder das ausdrücklich angeforderte `yolo` lesen. Sie stammen aus denselben Quellen wie der portable Skill; dessen separate Installation ist für MCP optional. Die Umsetzung erfolgt mit den Projekt- und Browser-Werkzeugen des Agents und den Berechtigungen des Hosts. Das Lesen einer Anleitung führt den Workflow nicht aus; Client-Unterstützung und Modellverhalten bestimmen, wie sie genutzt wird.

Natives Windows mit Claude Code: folge dessen [stdio-Setup-Hinweisen](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) und dem `cmd /c`-Wrapper für `npx`.

**Skill.** Der [figlens-Skill](../skills/figlens/SKILL.md) bringt einem Agent bei, Frames zu finden, das Bundle in kleinen Teilen zu lesen, begrenzte Arbeit zu planen und die laufende UI gegen das Design zu prüfen. Er installiert nur Anweisungen; die CLI oder der MCP-Server muss verfügbar sein. Installiere ihn mit der [Skills CLI](https://github.com/vercel-labs/skills) oder kopiere `skills/figlens/` in das Skills-Verzeichnis deines Agents (Claude Code: `~/.claude/skills/`):

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** Verwalte den Skill und die MCP-Verbindung für Claude Code, Codex und Cursor zentral mit [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**Installationshinweise.** Der npm-Launcher lädt beim ersten Aufruf das passende native Binary herunter, prüft es gegen die SHA-256-Prüfsummen des Releases und legt es im Cache ab. Skript-Installationen aktualisierst du mit `figlens update`, Homebrew-Installationen mit `brew upgrade figlens`. Die Binaries decken macOS (Apple Silicon, Intel), Linux (arm64, x86_64, statisch mit musl) und Windows (x86_64) ab.

<a id="commands"></a>

## CLI-Referenz

| Befehl | Ausgabe |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | Node-IDs, REST-Typen (wie in `nodes`) und Namen; eingerückter Text oder verschachteltes JSON mit Größe, Ausblendung und Auto-Layout-Modus jedes Nodes |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Suche über Layer-Namen und Text ohne Beachtung der Groß-/Kleinschreibung; Pfade, Größen und Vorschauen. Standardmäßig 20 Ergebnisse (1-200) |
| `search <file> <query> [--node ID] [--type T[,T]] [--limit N] [--cursor C] [--include-hidden] [--include-internal] [--mode text]` | Gerankte Suche über Layer-Namen und vollständigen Text mit angewendeten Instanz-Overrides, Text-/Boolean-Properties und Swaps; exakte Node-Referenzen (auch `I…;…`-Sublayer-IDs für `inspect`), Treffergründe, UTF-16-Spannen, Abdeckung und Cursor-Seiten. Standardmäßig 20 (1-200); ausgeblendete Layer und interne Seiten brauchen ihr Flag |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID]` | Kompakte Zusammenfassung des expandierten Layouts/Texts, höchstens 200 Nodes. `--images` ergänzt Bitmap-Abmessungen und Crop-Footprints |
| `nodes <file> <node>` | Das Objekt je Node einer REST-nodes-Antwort (`document`, `components`, `componentSets`, `styles`): expandierte Instanzen und Overrides, Auto-Layout, Textstile, Füllungen, Effekte, gebundene Variablen, Komponenten-Eigenschaften, Render-Bounds |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | Ein Decode, viele Nodes: eine JSON-Zeile pro gelisteter ID, die `nodes`-Ausgabe mit ihrer `nodeId` oder `{nodeId, error}` |
| `variables <file> [node]` | Die gebundenen Variablen eines Frames; ohne Node alle lokalen Variablen |
| `bundle <file> <node> <out.zip> [--states]` | Ein Design-Bundle für einen Frame; gibt `{nodeId, path, warnings}` aus |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | Viele Bundles aus einem Decode, parallel |
| `verify <bundle>` | Prüft ein Bundle (`.zip` oder entpacktes Verzeichnis) gegen sein Manifest: verifizierte, fehlende und geänderte Dateien, fehlende Assets, die im Projekt zu bestätigenden Fonts und `findings`: jede fehlende, nicht unterstützte oder unverifizierte Handoff-Eingabe mit Nodes, Belegen und nächstem Schritt |
| `export <file> <node> <out> [--bounds layout] [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP oder AVIF; `--max-kb` senkt die verlustbehaftete Qualität, bis es passt; `--bounds layout` exportiert die Layout-Box statt der Render-Grenzen |
| `export <file> --frames <list> --out-dir <dir> [--format F] [--scale N] ...`, `--frames -` (stdin) | Ein Decode, viele Exporte (Standard `png`): schreibt `<dir>/<nodeId>.<format>`, eine JSON-Zeile pro ID, die `export`-Ausgabe mit ihrer `nodeId` oder `{nodeId, error}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Lokaler schreibgeschützter Viewer; `--out` schreibt einen eigenständigen Offline-Snapshot |
| `compare <file> <nodes.json> <node>` | Diff gegen einen REST-Export desselben Frames |
| `node <file> <node>` | Ein roher `.fig`-Node |
| `mcp` | Lokaler MCP-Server über stdio |
| `update` | Ersetzt das Binary durch das neueste Release |

Für zugehörige Komponentenvarianten `inspect --states` oder `bundle --states` verwenden (MCP: `states: true`, standardmäßig false; keine Konfigurationsänderung). Die Nachweise behalten Quellen-, Unbekannt- und Kürzungsmarkierungen. Bundles ergänzen `design/states.json` und eine Zusammenfassung in `context.md`. Umfang und Grenzen stehen unter [Zustandsnachweise](../wiki/states.md).

Zwei Varianten desselben Component-Sets mit `inspect <file> <variantA> --compare-variant <variantB>` vergleichen (MCP `compareVariant`). `variantComparison` meldet Unterschiede gespeicherter Felder und Wertemengen samt Quell-IDs und unbekannten Werten, ohne Kindknoten einander zuzuordnen. Siehe [Vergleichsgrenzen](../wiki/states.md#compare-two-variants).

Ein Node-Argument akzeptiert `12:345`, `12-345` oder eine Figma-URL mit `?node-id=`. `<file>` kann auch `gs://bucket/object` sein, heruntergeladen mit deinem `gcloud`-Login und im Cache abgelegt. Der [GCS-Snapshot-Leitfaden](../wiki/gcs.md) beschreibt die Übergabe über ein festes Objekt, nötige Zugriffsrechte und Grenzen der Aktualisierung. Jeder CLI-Aufruf decodiert die Datei; MCP verwendet nur die zuletzt gelesene, unveränderte lokale Datei erneut. Remote-Aufrufe parsen erneut. Führe `figlens --help` oder `figlens inspect --help` (auch `-h`) für die vollständige CLI-Hilfe aus.

`inspect` enthält gespeicherte Textgrößenanpassung, Kürzung, maximale Zeilenzahl und Textdekoration. `handoffDiagnostics` meldet gespeicherte, aber nicht ausgegebene Semantik mit Anzahl, Beispielen und [Abdeckungsgrenzen](../wiki/semantics.md#known-handoff-gaps). Ausgaben schützen die Quelle und ersetzen das Ziel erst nach vollständigem Schreiben. MCP begrenzt Anfragezeilen auf 1 MiB und vollständige Antworten auf 8 MiB; Überschreitungen ergeben einen ausdrücklichen Fehler. Vollständiges JSON bleibt über die CLI verfügbar.
