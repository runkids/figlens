<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Dein Coding-Agent liest Figma-Dateien.<br>Keine Figma-App. Kein Login. Kein API-Kontingent.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="https://github.com/runkids/figlens/releases"><img src="https://img.shields.io/github/downloads/runkids/figlens/total?label=GitHub%20downloads&amp;color=79b9a5" alt="Total GitHub release asset downloads"></a>
  <a href="https://pi.dev/packages/@runkids/figlens"><img src="https://img.shields.io/badge/Pi-package-8b78c8" alt="Pi package"></a>
  <a href="#setup-opencode"><img src="https://img.shields.io/badge/OpenCode-V2-8b78c8" alt="OpenCode V2 plugin"></a>
  <a href="#setup-omp"><img src="https://img.shields.io/badge/OMP-plugin-79b9a5" alt="OMP plugin"></a>
  <a href="#mcp-and-skill"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#start">Start</a> · <a href="#demo">Demo</a> · <a href="#why">Warum</a> · <a href="#bundle">Was der Agent bekommt</a> · <a href="#pi">Pi</a> · <a href="#limitations">Einschränkungen</a>
</p>

**Das Problem.** Dein Agent braucht Layoutwerte, Text und Assets, um einen Bildschirm umzusetzen. Auf einem Remote-Host oder in CI muss dieser Designkontext neben dem Code verfügbar sein, ohne eine laufende Desktop-App vorauszusetzen.

**Die Lösung.** Wähle in Figma **File > Save local copy**. Lege `design.fig` neben deinen Code. figlens parst die Datei lokal und gibt deinem Agent Layout, Text, Stile, Variablen, Icons, Bilder und ein Referenz-Rendering jedes Frames. Ein natives Binary, keine Netzwerkaufrufe an Figma.

<a id="start"></a>

## Erste Schritte

**Noch keine Designdatei?** [Prüfe dein Setup mit dem kleinen öffentlichen Beispiel](examples/first-success/README.md): Verifiziere das gewählte Binary, finde einen Frame und prüfe sein Bundle. Kein Modell erforderlich.

Lege `design.fig` in dein Projekt und wähle dann **ein** Setup. Jedes gibt deinem Agent die figlens-Tools und dieselbe eingebaute Anleitung für Planung, Umsetzung und Bildaufbereitung.

| Setup | Geeignet für | Anleitung kommt von |
|---|---|---|
| [MCP](#setup-mcp) | Claude Code, Cursor, Codex, Gemini CLI und andere MCP-Clients | dem `workflow`-Tool des Servers |
| [Pi](#setup-pi) | Pi 1.3.0+ | dem Paket, das den Skill enthält |
| [OpenCode V2](#setup-opencode) | OpenCode V2 2.2.0 | dem mitgelieferten Skill |
| [OMP](#setup-omp) | OMP 18.13.2 | dem mitgelieferten Skill |
| [CLI](#setup-cli) | Agents, die nur Shell-Befehle ausführen, Skripte und CI, oder dich | `figlens workflow <topic>` oder dem optionalen [Skill](#setup-skill) |

<a id="setup-mcp"></a>

### MCP

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

**Codex CLI** (Node.js 18+, macOS/Linux):

```sh
codex mcp add figlens -- npx -y @runkids/figlens mcp
```

**Jeder MCP-Client:**

```json
{
  "mcpServers": {
    "figlens": {
      "command": "npx",
      "args": ["-y", "@runkids/figlens", "mcp"]
    }
  }
}
```

Ohne Node.js installierst du das eigenständige Binary ([CLI](#setup-cli)) und verwendest `"command": "figlens", "args": ["mcp"]`. Starte eine neue Session und prüfe, ob die figlens-Tools aufgelistet sind (`/mcp` in Claude Code). Eine separate Skill-Installation ist nicht nötig. Mit `--root <dir>` nach `mcp` begrenzt du, welche Ordner figlens lesen und schreiben darf.

<a id="setup-pi"></a>

### Pi

Ein Paket liefert CLI, MCP, Viewer, `/figlens`-Befehle und Skill:

```sh
pi install npm:@runkids/figlens
```

[Anleitung zur Pi-Erweiterung](docs/README-de.md#pi)

<a id="setup-opencode"></a>

### OpenCode V2

Zur Plugin-Konfiguration von OpenCode V2 hinzufügen:

```json
{ "plugins": ["@runkids/figlens"] }
```

Den Host neu starten und `/figlens help` ausführen. [OpenCode V2](opencode/README.md)

<a id="setup-omp"></a>

### OMP

Dasselbe Paket in OMP installieren:

```sh
omp plugin install @runkids/figlens
```

Den Host neu starten und `/figlens help` ausführen. [OMP](omp/README.md)

<a id="setup-cli"></a>

### CLI

Über npm ausführen oder das eigenständige Binary installieren:

```sh
npx @runkids/figlens --help                                                                # Node.js 18+, ohne Installation
curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh | sh  # macOS, Linux
brew install runkids/tap/figlens                                                           # Homebrew
irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 | iex      # Windows PowerShell
```

`npx` führt das Binary der aufgelösten Paketversion aus. Skript-Installationen aktualisierst du mit `figlens update`, Homebrew-Installationen mit `brew upgrade figlens`. Binaries gibt es für macOS (Apple silicon, Intel), Linux (arm64, x86_64) und Windows (x86_64).

Ein Agent, der die CLI nutzt, liest seine Anleitung mit `figlens workflow build` (außerdem `audit`, `images`, `plan`, `planning-details` und das nur auf ausdrücklichen Wunsch genutzte `yolo`), die immer zur Version des Binarys passt, oder aus dem [Agent-Skill](#setup-skill).

[CLI-Referenz](docs/README-de.md#commands)

<a id="setup-skill"></a>

### Agent-Skill (optional)

Der Skill bringt einem Agent denselben Ablauf bei wie die eingebaute Anleitung: Frame finden, das Bundle in kleinen Teilen lesen, den Umfang planen und die laufende UI mit dem Design vergleichen. Er enthält nur Anweisungen, also kombiniere ihn mit der CLI oder MCP. MCP- und Pi-Setups brauchen ihn nicht.

**Empfohlen: [skillshare](https://github.com/runkids/skillshare).** Installiere den Skill einmal und synchronisiere ihn mit jedem Agent, den du nutzt:

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

Dasselbe Tool kann für diese Agents auch die figlens-MCP-Verbindung einrichten:

```sh
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

Weitere Wege, den Skill zu installieren:

```sh
npx skills add runkids/figlens --skill figlens   # Skills CLI
```

Oder kopiere `skills/figlens/` aus diesem Repository oder einem Release-Archiv in das Skills-Verzeichnis deines Agents (Claude Code: `~/.claude/skills/`). Ein installierter Skill wird getrennt vom Binary aktualisiert; `figlens workflow` passt immer zum Binary.

Bitte dann um einen Bildschirm:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

Der Agent findet den Frame, bündelt Designdaten, Assets und Referenzbild und setzt den Bildschirm mit deinem Stack um. Der gemeinsame Workflow leitet die Prüfung von Schriften und Assets an; fehlende Ressourcen werden gemeldet, während die Layout-Implementierung mit dokumentierten Alternativen weitergeht. Lass dir die geänderten Dateien und einen Browservergleich zeigen.

<a id="demo"></a>

## Demo: Pi baut einen React-Bildschirm aus einer .fig

[![Pi plant aus einer lokalen .fig, baut einen React-Bildschirm und prüft ihn neben einem Live-Browser](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

Die GIF-Schleife läuft mit der dreifachen Wiedergabegeschwindigkeit des geschnittenen Videos. Anklicken für das vollständige 78-Sekunden-Video.

**[78-Sekunden-Video](assets/taskflow-demo.mp4).** Pi führt `/figlens plan` aus, liest einen Dashboard-Frame aus einer lokalen `.fig`, baut ihn mit React und Tailwind CSS und prüft und korrigiert dann die laufende UI. Geschnittener Zeitraffer, kein Benchmark und kein Pixel-Perfect-Versprechen. Design: [Task Flow von Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [Aufnahmenotizen](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

**[Von .fig zu editierbarem HTML/CSS](examples/task-list/README.md):** In einer kontrollierten Aufgabenlisten-Studie behielten alle sechs figlens-basierten Versuche alle 19 sichtbaren Textvorkommen als echten DOM-Text und alle vier Icons bei. Auch CLI- und MCP-Extraktion bestanden mit deaktiviertem Docker-Netzwerk. Die Fallstudie enthält Einrichtung, Messwerte und unabhängige Bildvergleiche.

<a id="why"></a>

## Warum figlens statt des offiziellen Figma MCP?

Nutze **figlens**, wenn das Design als Datei wandern soll: Remote-Agents, CI, Offline-Arbeit, Übergabe zwischen Agents, unbegrenzt wiederholte Lesezugriffe. Nutze den **offiziellen Figma MCP**, wenn der Agent das Live-Design ohne Export-Schritt sehen muss.

Eine `.fig` zu lesen funktioniert offline. Installieren und Aktualisieren von figlens, `gs://`-Eingaben, dein Modellanbieter und die Abhängigkeiten deines Projekts brauchen weiterhin ein Netzwerk.

[Vollständiger Vergleich](docs/README-de.md#why)

<a id="bundle"></a>

## Was der Agent bekommt

Ein Frame, eine Zip. Struktur zum Bauen, Assets zum Verwenden, ein Bild zum Abgleichen.

[Bundle-Inhalt und Beispiele](docs/README-de.md#bundle) · [Wie nah ist es an Figma?](docs/README-de.md#accuracy)

## Dokumentation

- <a id="pi"></a>[Pi-Extension](docs/README-de.md#pi)
- <a id="viewer"></a>[Ein Design ohne Figma durchsuchen](docs/README-de.md#viewer)
- <a id="mcp-and-skill"></a>[MCP-Server und Agent-Skill](docs/README-de.md#mcp-and-skill)
- <a id="commands"></a>[CLI-Referenz](docs/README-de.md#commands)

<a id="limitations"></a>

## Einschränkungen

- **Snapshots, keine Live-Synchronisierung.** Exportiere eine neue `.fig`, wenn sich das Design ändert. figlens kann kein Design über eine Figma-URL herunterladen.
- **Undokumentiertes Format.** figlens decodiert das in der Datei eingebettete Schema; neue Figma-Exporte können ein figlens-Update erfordern.
- **Schriften und Text:** Schriftdateien sind nicht enthalten; überschriebener Text kann Figmas gespeichertes Layout behalten. Verwende lizenzierte Projektschriften und vergleiche das Ergebnis.
- **Unterstützungsumfang:** [Rendering-Verbesserungen](https://github.com/runkids/figlens/issues/36) und [Handoff-Semantik](https://github.com/runkids/figlens/issues/37) werden im Issue-Tracker verfolgt. Siehe die [aktuelle semantische Abdeckung](wiki/semantics.md).
- **Prüfe die Ausgabe.** Lies die Bundle-`warnings`, `missing-assets.json` und ungelöste Variablen in `context.md` und vergleiche dann die Implementierung mit dem Design.
- Bekannte sichtbare Render-Grenzen stehen in `_figlens.renderDiagnostics` der Knoten, begrenzten `inspect`-Zusammenfassungen und Bundle-`warnings`; `missing-assets.json` enthält nur Asset-Fehler. Auch ohne Warnungen ist Pixeltreue nicht garantiert.

## Hilf figlens zu wachsen

Wenn figlens für dich ein Design in Code verwandelt hat, [gib dem Repo einen Stern](https://github.com/runkids/figlens), damit andere es finden. Eine Datei, die figlens nicht lesen kann? [Eröffne ein Issue](https://github.com/runkids/figlens/issues) mit dem Befehl und deiner Erwartung.

Die Entwicklung führt Rust ausschließlich in Docker aus: `docker compose run --rm dev cargo test`. Siehe [AGENTS.md](AGENTS.md) und das [Wiki](wiki/README.md).

## Lizenz

[MIT](LICENSE). figlens ist ein unabhängiges Projekt und steht in keiner Verbindung zu Figma, wird nicht von Figma unterstützt und nicht von Figma gesponsert. Das `.fig`-Format ist undokumentiert und kann sich ändern; verwende nur Dateien und Assets, auf die du zugreifen und die du teilen darfst.
