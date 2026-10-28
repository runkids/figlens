# figlens — 사용자 가이드

[← README](../README-ko.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [독립 바이너리 설치](#install)
- [공식 Figma MCP 대신 figlens를 쓰는 이유](#why)
- [에이전트가 받는 것](#bundle)
- [Pi 확장](#pi)
- [Figma 없이 디자인 둘러보기](#viewer)
- [MCP 서버와 에이전트 skill](#mcp-and-skill)
- [CLI 레퍼런스](#commands)

<a id="install"></a>

## 독립 바이너리 설치

**Node.js가 없나요?** 독립 바이너리를 설치하고 `"command": "figlens", "args": ["mcp"]`를 대신 사용하세요.

| 플랫폼 | 명령어 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

<a id="why"></a>

## 공식 Figma MCP 대신 figlens를 쓰는 이유

디자인을 파일로 전달해야 할 때는 **figlens**를 사용하세요. 원격 에이전트, CI, 오프라인 작업, 에이전트 간 전달, 제한 없는 반복 읽기가 해당합니다. 내보내기 없이 에이전트가 실시간 디자인을 봐야 할 때는 **공식 Figma MCP**를 사용하세요.

| | figlens | 공식 Figma MCP |
|---|---|---|
| 소스 | 내보낸 `.fig` 스냅샷 | 실시간 Figma 파일 |
| 필요한 것 | 파일 | Figma 로그인. 데스크톱 서버는 앱 필요 |
| Rate limit | 없음, 로컬 파싱 | Figma 서비스 한도 |
| 원격 에이전트 / CI | 파일을 복사하고 바이너리 실행 | 로그인이 필요한 원격 서버 |
| 디자인 변경 | 새 `.fig` 내보내기 | 자동 |
| 전달 | `.fig` 또는 frame bundle 공유 | Figma 접근 권한 공유 |
| 충실도 | [알려진 차이](../README-ko.md#limitations)가 있는 독립 파서 | Figma가 유지 관리 |

`.fig`를 읽는 일은 오프라인입니다. figlens 설치와 업데이트, `gs://` 입력, 모델 제공업체, 프로젝트 의존성에는 여전히 네트워크가 필요합니다.

<a id="bundle"></a>

## 에이전트가 받는 것

frame 하나, zip 하나. 구현할 구조, 사용할 에셋, 비교할 이미지가 담겨 있습니다.

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

일부만 필요한가요? CLI로 바로 가져올 수 있습니다.

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # 이름 또는 텍스트로 frame id 찾기
npx -y @runkids/figlens inspect design.fig 12:345                 # 간결한 레이아웃·텍스트 요약
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # REST 형태 JSON
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP 또는 AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # 위의 zip
```

SVG 내보내기는 원본 이미지 채우기를 포함하고 텍스트를 글리프 윤곽선으로 변환하므로 글꼴이 필요 없습니다. MCP에서는 래스터 내보내기도 인라인 이미지로 반환되어 에이전트가 파일을 열지 않고 디자인을 볼 수 있습니다.

<a id="accuracy"></a>

### Figma와 얼마나 가깝나요?

Figma 자체의 JSON, PNG, SVG 내보내기와 비교해 **실제 서비스 frame 826개**와 Community 파일 5개에서 측정했습니다(2026-10-22).

| 검증 항목 | 결과 |
|---|---|
| 노드 구조, 이름, 표시 여부, 경계, 텍스트 | 826개 중 771개 frame에서 동일 |
| 원본 이미지 채우기 | 2,608 / 2,608개가 바이트 단위로 동일 |
| SVG 에셋 | 5,014 / 5,197개가 픽셀 차이 임계값 이내 |
| Community frame bundle | 306 / 306개에서 누락 에셋 없음 |

해당 파일에 대한 결과이며 모든 디자인에 대한 보장은 아닙니다. 방법과 임계값: [정확도 보고서](../wiki/accuracy.md).

<a id="pi"></a>

## Pi 확장

```sh
pi install npm:@runkids/figlens
```

Pi를 다시 시작하거나 `/reload`를 실행한 뒤 평소 말로 화면을 요청하세요. Pi는 frame을 찾고, 자동 등록된 MCP 서버로 읽고, 프로젝트의 기술 스택으로 구현합니다. 노드 ID도, 전역 CLI도 필요 없습니다.

- `/figlens plan <request>`는 계획만 작성하며 애플리케이션을 수정하지 않습니다.
- `/figlens yolo <request>`는 계획, 구현, 확인을 하고 자신이 만든 임시 계획을 정리합니다.
- `/figlens preview [description]`는 로컬 viewer를 엽니다.
- `/figlens review`는 에이전트가 제안한 재사용 컴포넌트 매핑을 승인합니다.

![Pi에게 화면을 요청하면 디자인을 찾고, 기술 스택으로 구현하고, 결과를 확인합니다](../assets/pi-workflow.svg)

전체 안내, 명령어, 제한: [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Figma 없이 디자인 둘러보기

[![레이어 트리, 레이아웃, 컴포넌트 속성과 함께 MCP Apps 디자인을 로컬에서 확인하는 모습](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # 로컬 viewer, Ctrl+C로 종료
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # 누구에게나 보낼 수 있는 오프라인 HTML 스냅샷
```

페이지와 레이어를 탐색하고, 검색하고, 거리를 측정하고, 레이아웃, 타이포그래피, 변수, CSS를 확인하고, 모든 레이어를 내보내고, 구현 프롬프트를 복사하거나 선택 영역의 bundle을 다운로드할 수 있습니다. 구현 결과의 스크린샷을 가져오면 디자인 위에 겹쳐 볼 수 있습니다. 자세한 내용: [미리보기 가이드](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## MCP 서버와 에이전트 skill

**MCP.** 서버는 로컬에서 stdio로 실행되며 `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `verify`, `export`를 제공합니다. 절대 경로를 전달하세요. `--root /path/to/designs`(반복 가능)를 추가하면 모든 파일과 출력 경로를 선택한 폴더로 제한합니다. 래스터 내보내기는 `maxSize`(기본 1600px)에 맞게 축소되어 인라인 이미지가 작게 유지되며, SVG는 메타데이터만 반환합니다.

agent는 `workflow`로 `build`, `audit`, `images`, `plan`, `planning-details` 또는 명시적으로 요청된 경우 `yolo` 가이드를 읽을 수 있습니다. portable skill과 같은 소스를 사용하므로 MCP 사용 시 별도 skill 설치는 선택 사항입니다. 실제 작업은 agent의 프로젝트·브라우저 도구와 호스트 권한으로 수행합니다. 가이드를 읽는 것만으로 작업이 실행되지는 않으며, 활용 방식은 클라이언트 지원과 모델 동작에 따라 달라집니다.

Claude Code를 네이티브 Windows에서 사용한다면 [stdio 설정 안내](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server)와 `npx`용 `cmd /c` 래퍼를 따르세요.

**Skill.** [figlens skill](../skills/figlens/SKILL.md)은 에이전트가 frame을 찾고, bundle을 작게 나누어 읽고, 범위가 정해진 작업을 계획하고, 실행 중인 UI를 디자인과 대조해 검증하도록 안내합니다. 지침만 설치하므로 CLI 또는 MCP 서버를 사용할 수 있어야 합니다. [Skills CLI](https://github.com/vercel-labs/skills)로 설치하거나 `skills/figlens/`를 에이전트의 skills 디렉터리(Claude Code는 `~/.claude/skills/`)에 복사하세요.

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** [skillshare](https://github.com/runkids/skillshare)로 Claude Code, Codex, Cursor 전반의 skill과 MCP 연결을 한곳에서 관리하세요.

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**설치 참고.** npm 런처는 처음 사용할 때 해당 네이티브 바이너리를 내려받고, 릴리스의 SHA-256 체크섬으로 검증한 뒤 캐시합니다. 스크립트로 설치했다면 `figlens update`, Homebrew로 설치했다면 `brew upgrade figlens`로 업데이트하세요. 바이너리는 macOS(Apple silicon, Intel), Linux(arm64, x86_64, 정적 musl), Windows(x86_64)를 지원합니다.

<a id="commands"></a>

## CLI 레퍼런스

| 명령어 | 출력 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 노드 ID, 유형, 이름. 들여쓰기 텍스트 또는 중첩 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 대소문자 구분 없이 레이어 이름과 텍스트 검색. 경로, 크기, 미리보기. 기본 20개(1-200) |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID]` | 펼쳐진 간결한 레이아웃/텍스트 요약, 최대 200노드. `--images`는 비트맵 크기와 크롭 범위를 추가 |
| `nodes <file> <node>` | REST 형태 JSON: 펼쳐진 인스턴스와 재정의, 자동 레이아웃, 텍스트 스타일, 채우기, 효과, 연결 변수, 컴포넌트 속성, 렌더링 경계 |
| `variables <file> [node]` | frame에 연결된 변수. 노드 없이 실행하면 모든 로컬 변수 |
| `bundle <file> <node> <out.zip> [--states]` | 한 frame의 디자인 bundle. `{nodeId, path, warnings}` 출력 |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | 한 번 디코딩해 여러 bundle을 병렬 생성 |
| `verify <bundle>` | bundle(`.zip` 또는 압축을 푼 디렉터리)을 manifest와 대조: 검증·누락·변경된 파일, 누락된 asset, 프로젝트에서 확인할 폰트 |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP, AVIF. `--max-kb`는 크기에 맞을 때까지 손실 품질을 낮춤 |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 로컬 읽기 전용 viewer. `--out`은 독립 오프라인 스냅샷 생성 |
| `compare <file> <nodes.json> <node>` | 같은 frame의 REST 내보내기와 차이 비교 |
| `node <file> <node>` | 원시 `.fig` 노드 하나 |
| `mcp` | stdio를 사용하는 로컬 MCP 서버 |
| `update` | 최신 릴리스로 바이너리 교체 |

관련 컴포넌트 variants는 `inspect --states` 또는 `bundle --states`로 확인합니다(MCP는 `states: true`, 기본값 false, 설정 변경 불필요). 근거에는 출처, 알 수 없는 값, 생략 표시가 유지됩니다. Bundle에는 `design/states.json`과 `context.md` 요약이 추가됩니다. 범위와 제한은 [상태 근거](../wiki/states.md)를 참조하세요.

같은 component set의 두 variants는 `inspect <file> <variantA> --compare-variant <variantB>`(MCP `compareVariant`)로 비교합니다. `variantComparison`은 저장된 필드와 값 집합의 차이, 출처 ID, 알 수 없는 항목을 반환하며 자식 노드의 대응 관계는 추측하지 않습니다. [비교 제한](../wiki/states.md#compare-two-variants)을 참조하세요.

노드 인수는 `12:345`, `12-345`, `?node-id=`가 있는 Figma URL을 받을 수 있습니다. `<file>`에 `gs://bucket/object`도 지정할 수 있으며 `gcloud` 로그인으로 다운로드하고 캐시합니다. 고정 객체를 통한 전달, 필요한 접근 권한과 갱신 제한은 [GCS 스냅샷 가이드](../wiki/gcs.md)를 참고하세요. CLI 호출은 매번 파일을 디코딩합니다. MCP는 마지막으로 읽은 변경 없는 로컬 파일만 재사용합니다. 원격 입력은 다시 파싱합니다. 전체 CLI 사용법은 `figlens --help` 또는 `figlens inspect --help`(`-h`도 가능)를 실행하세요.

`inspect`에는 기록된 텍스트 크기 조정, 잘림, 최대 줄 수와 장식이 포함됩니다. `handoffDiagnostics`는 저장되었지만 출력되지 않는 의미 정보를 개수, 예시, [범위 제한](../wiki/semantics.md#known-handoff-gaps)과 함께 보고합니다. 출력은 원본을 보호하고 쓰기가 완료된 뒤 대상을 교체합니다. MCP 요청 행은 1 MiB, 전체 응답은 8 MiB로 제한되며 초과하면 명시적인 오류를 반환합니다. CLI에서는 전체 JSON을 계속 출력할 수 있습니다.
