<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Figma 디자인을 코딩 에이전트에게 바로 전달하세요.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  어떤 화면이든 레이아웃, 텍스트, 변수, 에셋, 참조 이미지를 코딩 에이전트에게 전달하세요.<br>
  노트북에서도, 클라우드에서도, CI에서도 원하는 만큼 읽을 수 있습니다.<br>
  <strong>작업하는 컴퓨터에 Figma 앱, 로그인, 시트가 필요 없고, API 호출 제한도 없습니다.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">데모 보기</a> · <a href="#quick-start">내 디자인으로 시작</a> · <a href="#for-ai-agents">에이전트 연결</a> · <a href="#pi-extension">Pi와 함께 사용</a> · <a href="#how-close-is-it-to-figma">정확도</a>
</p>

<a id="why-figlens"></a>

## 왜 figlens인가요?

“이 화면을 만들어 줘”라는 요청에는 화면의 레이아웃, 텍스트, 에셋도 함께 전달되어야 합니다. 스크린샷으로는 에이전트가 간격과 타이포그래피를 추정해야 하고, 아이콘은 여전히 손으로 내보내야 합니다. Figma MCP 서버는 실제 디자인을 읽지만, 할당량은 파일을 소유한 팀에 묶여 있고, 에이전트가 프롬프트 하나로 하루치 할당량을 다 쓸 수 있으며, 컴퓨터에 Figma 로그인이 필요합니다.

figlens는 대신 저장된 사본에서 디자인을 읽습니다. Figma에서 **File > Save local copy**를 선택하세요(소유자가 복사를 막지 않았다면 보기 권한으로 충분합니다). 그 `.fig`를 CLI나 로컬 MCP 서버를 통해 에이전트에게 주면 화면을 찾고 레이아웃과 텍스트를 확인하며 필요한 에셋을 추출할 수 있습니다. 또는 프레임 하나의 디자인 데이터, 에셋, 참조 렌더링을 어떤 에이전트나 팀원도 쓸 수 있는 zip으로 묶을 수 있습니다.

| 이런 문제를 겪었다면 | figlens로 할 수 있는 일 |
|---|---|
| 유료 시트인데도 “Rate limit exceeded, try again tomorrow”가 뜸 | Figma API 호출, 토큰, 시트 없이 저장된 파일을 필요한 만큼 읽기 |
| 에이전트가 클라우드 서버나 CI에서 실행됨 | Figma 앱이나 브라우저 세션 없이 네이티브 CLI로 디자인 읽기 |
| 치수를 설명하고 에셋을 내보내는 일을 반복함 | 구조화된 레이아웃, 타이포그래피, 펼쳐진 인스턴스, 연결된 변수, 원본 에셋 제공 |
| 다른 에이전트나 팀원도 같은 화면이 필요함 | 디자인 데이터, 에셋, 참조 렌더링, 지침이 담긴 프레임 번들 공유 |

디자이너는 계속 Figma에서 작업하고, 에이전트는 프로젝트의 기존 기술 스택을 사용합니다. figlens는 디자인 컨텍스트를 제공하고, 코딩 에이전트가 구현 코드를 작성합니다.

<details>
<summary>.fig 형식은 문서화되어 있지 않습니다. Figma가 형식을 바꾸면 어떻게 되나요?</summary>

figlens는 형식을 추측하지 않습니다. 모든 릴리스는 같은 파일을 Figma가 직접 내보낸 JSON, PNG, SVG와 대조하므로([결과](#how-close-is-it-to-figma)) 형식 변경은 조용히 묻히지 않고 측정된 회귀로 드러납니다. 이미 저장해 둔 사본은 그것을 읽은 figlens 버전에서 계속 동작합니다. 바뀐 Figma에서 새로 저장한 파일만 figlens 업데이트가 필요하며, `figlens update`로 받을 수 있습니다.

</details>

<a id="from-a-local-fig-to-working-html"></a>

## 로컬 .fig에서 작동하는 HTML까지

[![Pi가 로컬 Figma 파일로 작동하는 인터페이스를 만들고 터미널 옆에서 미리보기를 표시하는 모습](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[영상 보기](assets/demo-8x.mp4)** · **8배속**으로 재생하는 45초 타임랩스입니다. Orca에서 진행한 실제 Pi 세션으로, 로컬 디자인을 읽고 HTML/CSS를 만든 뒤 실시간 미리보기에서 공유 대화상자와 입력 영역을 사용합니다. HTML 인터페이스에는 번들의 SVG 아이콘 19개를 사용했습니다.

| 이 예제 | 측정 결과 |
|---|---|
| 로컬 `.fig` 파일 | 57.5 MiB |
| 선택한 프레임 | 노드 153개, SVG 에셋 25개, 누락 에셋 없음 |
| 디자인 번들 추출 | 웜 캐시로 5회 실행한 중앙값 **0.47초** |

측정 대상은 `figlens bundle`이며, **AI의 구현 시간은 포함하지 않습니다**. ARM64 Docker 컨테이너에서 figlens 0.1.2로 측정했습니다. [명령어, 개별 결과, 녹화 설명](wiki/accuracy.md#local-design-to-html-demo)을 참고하세요. Pi가 코드를 작성하고 figlens가 디자인 데이터를 제공합니다. AI 제공업체에는 여전히 네트워크로 연결합니다.

<details>
<summary>디자인 출처와 데모의 제약</summary>

“List views” 프레임의 출처는 [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude)입니다. 원본 Anthropic 글꼴은 포함되지 않아 시스템 sans와 Georgia를 사용했습니다. Share는 로컬 대화상자를 열고, 답글은 페이지에 남지만 새로고침하면 사라집니다. 실제 Orca 창의 연속 스크린샷으로 녹화해 일반적인 동영상 캡처보다 움직임이 덜 부드럽습니다.

</details>

<a id="quick-start"></a>

## 빠른 시작

<a id="1-open-your-design-locally"></a>

### 1. 로컬에서 디자인 열기

Figma에서 **File > Save local copy**를 선택해 `design.fig`로 저장한 뒤 실행하세요.

```sh
npx @runkids/figlens preview design.fig
```

터미널에 표시된 로컬 URL을 여세요. 페이지를 둘러보고 구현할 화면을 선택하면 간격, 타이포그래피, 변수, 에셋을 바로 확인할 수 있습니다.

Node.js 18+가 필요합니다. 첫 실행 시 네이티브 바이너리를 내려받아 검증하고 캐시합니다. [독립 설치](#installation)는 Node.js, Rust, Docker가 필요 없습니다. Ctrl+C로 뷰어를 종료합니다.

<a id="2-give-the-screen-to-your-agent"></a>

### 2. 에이전트에게 화면 전달하기

뷰어에서 선택한 프레임의 번들을 다운로드하세요. zip과 다음 지침을 코딩 에이전트에게 전달하세요.

> 이 번들을 별도 디렉터리에 압축 해제하고 context.md를 읽어 주세요. 이 프로젝트의 기술 스택과 규칙에 맞게 화면을 구현하세요. 제공된 레이아웃 값과 에셋을 사용하고 references/screen.png와 비교해 결과를 확인하세요. 누락된 에셋이나 사용할 수 없는 글꼴을 보고하세요.

번들에는 구조, 에셋, 참조 이미지가 함께 있어 에이전트가 일반 파일 도구로 읽을 수 있습니다. 화면을 직접 찾고 확인하게 하려면 [CLI나 MCP 서버를 연결](#for-ai-agents)하세요. [Pi에서는 화면 이름으로 요청할 수 있습니다](#pi-extension).

<a id="3-check-the-implementation"></a>

### 3. 구현 확인하기

구현한 화면의 스크린샷을 뷰어에 가져오세요. 디자인 옆에 나란히 놓거나 투명도를 조절해 겹쳐 보면서 차이를 찾을 수 있습니다. 스크린샷은 브라우저 안에만 남습니다. 이는 시각적 확인이며, figlens가 구현의 정확도 점수를 계산하지는 않습니다.

**`.fig`는 스냅샷입니다.** 디자인이 바뀌면 새 로컬 사본을 저장하세요. Figma URL은 그 사본 안의 노드를 선택할 수 있지만 디자인을 다운로드하지는 않습니다. 설치 후 파일 읽기는 오프라인으로 작동합니다. AI 제공업체나 프로젝트 의존성에는 여전히 네트워크가 필요할 수 있습니다.

<details>
<summary>터미널을 선호하나요? 프레임을 찾아 바로 번들로 묶으세요.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

`12:345`를 트리나 검색 결과의 ID로 바꾸세요. 노드는 ID(`12:345`), URL 형식(`12-345`), `?node-id=`가 있는 Figma URL로 지정할 수 있습니다. `find`로 레이어 이름이나 텍스트를 검색하고, `inspect`로 펼쳐진 인스턴스 자식과 간결한 레이아웃/텍스트 요약을 확인하세요. 자세한 내용은 [명령어](#commands)를 참고하세요.

</details>

<a id="browse-and-hand-off-a-design"></a>

## 디자인 둘러보기와 전달하기

[![레이어 트리, 레이아웃, 컴포넌트 속성과 함께 MCP Apps 디자인을 로컬에서 확인하는 모습](assets/preview-inspector.png)](assets/pi-demo.mp4)

로컬 뷰어로 Figma 세션 없이 저장된 디자인을 살펴볼 수 있습니다.

- 페이지와 레이어를 탐색하고, 이름과 텍스트를 검색하고, 거리를 측정하고, 레이아웃·타이포그래피·변수·컴포넌트 속성·CSS를 확인합니다.
- 선택한 레이어를 SVG, PNG, JPG, WebP, AVIF로 내보내거나 원본 이미지 채우기를 다운로드합니다.
- 레이어 ID, 구조화된 컨텍스트, 구현 지침을 복사하고, 인스턴스 하위 레이어를 포함한 정확한 선택 범위의 번들을 다운로드합니다.

다른 사람이 둘러볼 수 있는 디자인을 보내려면 독립 HTML 스냅샷을 내보내세요.

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

서버나 네트워크 없이 열리며 디자인 데이터, 이미지, 스냅샷 범위의 번들을 포함합니다. 해당 에셋을 볼 권한이 있는 사람에게만 공유하세요. 자식 노드를 선택하면 확인하는 컨텍스트가 바뀌지만, 내장 번들은 원래 스냅샷 범위를 유지합니다.

실시간 뷰어는 저장된 로컬 파일을 감시하며 저장에 성공하면 갱신합니다. 일부만 저장되거나 저장에 실패하면 오래된 화면임을 표시합니다. HTML 스냅샷은 고정됩니다. 조작법, 캐시 관리, 브라우저 에이전트 자동화, 제약은 [미리보기 가이드](wiki/preview.md)를 참고하세요.

<a id="pi-extension"></a>

## Pi 확장

**Pi에게 `.fig`를 주고 원하는 화면을 요청하세요.** 패키지에는 CLI, MCP, 미리보기, 확장, 에이전트 skill이 포함됩니다.

```sh
pi install npm:@runkids/figlens
```

Pi를 다시 시작하거나 `/reload`를 실행한 뒤 평소처럼 요청하세요.

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi는 화면을 찾고 레이아웃과 에셋을 읽고 프로젝트를 살펴본 뒤 기존 기술 스택으로 구현합니다. 프레임 ID를 알거나 `/figlens open`을 실행할 필요가 없습니다. 이름이 모호하면 몇 가지 후보를 보여줄 수 있습니다. MCP는 자동으로 시작하며 전역 CLI 설치는 필요 없습니다.

![Pi에게 화면을 요청하면 디자인을 찾고 기존 기술 스택으로 구현한 뒤 결과를 확인합니다](assets/pi-workflow.svg)

**선택적 컴포넌트 재사용:** Pi에게 재사용할 매핑을 저장하도록 요청하고, `/figlens review`로 프로젝트의 대기 중인 제안을 승인하거나 수정하세요. 결정은 `.figlens/component-mappings.md`에 보관됩니다. 변형이나 기록된 코드가 바뀌면 다시 검토해야 합니다. 화면 구현에 매핑이 필수는 아닙니다.

[![로컬 디자인을 살펴보고 Pi에서 기존 HTML 컴포넌트를 검토한 뒤 다음 프레임에 재사용하는 모습](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[선택적 매핑 흐름 보기](assets/pi-demo.mp4)** · v0.3.0의 20초 편집 영상입니다. MCP Apps 디자인을 확인하고, 준비된 기존 HTML 매핑을 승인한 뒤 새 세션에서 재사용합니다. 모델 생성은 보여주거나 시간을 측정하지 않았습니다. [가이드, 명령어, 제약](pi/README.md) · [녹화와 검증 설명](wiki/pi.md#demo-and-verification).

Pi 1.3.0+가 필요합니다. npm 확장은 포함된 CLI 런처를 사용합니다. MCP 자동 시작을 포함한 첫 실행 시 해당 네이티브 바이너리를 내려받아 검증합니다. Homebrew와 릴리스 압축 파일은 네이티브 CLI와 뷰어를 제공합니다. 개발 설정은 [소스 체크아웃 설정](pi/README.md#install-once)을 참고하세요.

<a id="the-design-bundle"></a>

## 디자인 번들

프레임 하나를 zip 하나로 전달합니다. 구현할 구조, 사용할 에셋, 비교할 이미지가 담겨 있습니다.

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

일부만 필요하다면 JSON을 내보내거나 노드를 직접 렌더링하세요.

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

SVG 내보내기는 프레임, 그룹, 아이콘, 정확한 인스턴스 하위 레이어를 지원합니다. 선택한 하위 트리는 원본 이미지 채우기와 글리프 윤곽선 텍스트를 포함한 독립 SVG가 됩니다. 번들 압축 해제는 필요 없습니다. SVG는 원래 크기를 유지하며 `--scale`과 `--quality`는 래스터 형식에만 적용됩니다.

<a id="how-close-is-it-to-figma"></a>

## Figma와 얼마나 일치하나요?

figlens는 Figma 자체의 JSON, PNG, SVG 내보내기와 비교해 검증합니다. **실제 서비스 프레임 826개**와 **Community 파일 5개**에서 기록한 결과는 다음과 같습니다.

| 검증 항목 | 결과 |
|---|---|
| 노드 구조, 이름, 표시 여부, 경계, 텍스트 | 실제 서비스 프레임 771 / 826개에서 동일 |
| 원본 이미지 채우기 | 2,608 / 2,608개가 바이트 단위로 동일 |
| SVG 에셋 | 5,014 / 5,197개가 픽셀 차이 임계값 이내 |
| Community 프레임 번들 | 306 / 306개에서 누락 에셋 없음 |

해당 파일에 대한 측정값이며 모든 디자인에 대한 보장은 아닙니다. 날짜, 임계값, 렌더링 결과, 방법은 [정확도 보고서](wiki/accuracy.md)를 참고하세요.

**알려진 제약:** constraints, layoutAlign/layoutGrow, 레이아웃 그리드, 컴포넌트 속성 정의는 아직 출력하지 않습니다. 렌더링은 배경 흐림과 일부 이미지 조정(대비, 색온도, 색조, 하이라이트, 그림자)을 생략하고, 각도 및 다이아몬드 그라디언트를 방사형으로 그립니다. 텍스트는 저장된 글리프 윤곽선을 사용해 렌더링에 글꼴 파일이 필요 없지만, 재정의된 텍스트에 Figma의 마지막 레이아웃이 남을 수 있습니다. 구현용 글꼴 파일은 포함되지 않습니다.

<a id="for-ai-agents"></a>

## AI 에이전트용

에이전트 skill로 CLI를 사용하거나 같은 바이너리를 MCP 서버로 연결하세요.

<a id="agent-skill"></a>

### 에이전트 skill

[figlens skill](skills/figlens/SKILL.md)은 프레임을 찾고 번들을 나누어 읽으며 그 값을 바탕으로 구현하는 방법을 에이전트에게 알려줍니다.

[Skills CLI](https://github.com/vercel-labs/skills)로 설치하세요.

```sh
npx skills add runkids/figlens --skill figlens
```

Claude Code, Codex, Cursor, Gemini CLI 등에서 skills를 관리하고 업데이트하려면 [skillshare](https://github.com/runkids/skillshare)를 사용하세요.

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all`로 최신 상태를 유지합니다. `skills/figlens/`를 에이전트의 skills 디렉터리(Claude Code는 `~/.claude/skills/`)에 복사할 수도 있습니다. 릴리스 압축 파일에도 포함됩니다.

<a id="mcp-server"></a>

### MCP 서버

[skillshare](https://github.com/runkids/skillshare)로 MCP 서버와 에이전트 skills를 함께 관리하세요. figlens를 한 번 정의하고 연결 설정을 Claude Code, Codex, Cursor에 동기화합니다.

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

다른 클라이언트와 기존 설정 가져오기는 [Skillshare MCP 가이드](https://skillshare.runkids.cc/docs/reference/commands/mcp/)를 참고하세요.

Claude Code에 직접 설정하려면:

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

다른 MCP 클라이언트:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

`figlens`를 독립 설치한 경우:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

서버는 로컬에서 stdio로 실행되며 `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle`, `export`를 제공합니다. 도구에는 절대 경로를 전달하세요. 상대 경로는 클라이언트가 figlens를 시작한 디렉터리를 기준으로 해석합니다.

먼저 `find`와 `inspect`로 프레임을 선택하세요. `export`는 파일을 저장하고 메타데이터를 반환합니다. 래스터 형식은 MCP 이미지 콘텐츠도 반환하므로 에이전트가 로컬 파일을 열지 않고 디자인을 볼 수 있습니다. SVG는 메타데이터만 반환합니다. PNG는 클라이언트 지원이 가장 넓으며 큰 프레임에는 `scale`을 줄이세요. `find`, `inspect`, `bundle`, `export`는 구조화된 JSON도 반환합니다.

<a id="installation"></a>

## 설치

Node.js 18+에서 `npx @runkids/figlens`를 사용하거나 독립 CLI를 설치하세요.

| 플랫폼 | 명령어 |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

릴리스 바이너리는 macOS(Apple silicon, Intel), Linux(arm64, x86_64, 정적 musl), Windows(x86_64, ARM64는 에뮬레이션)를 지원합니다. 독립 설치는 `figlens update`로 업데이트합니다.

<a id="commands"></a>

## 명령어

| 명령어 | 출력 |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | 노드 ID, 유형, 이름. 들여쓰기 텍스트(에이전트 컨텍스트 절약), `--json`이면 중첩 JSON |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | 대소문자 구분 없이 레이어 이름과 텍스트의 부분 문자열 검색. 경로, 크기, 미리보기. 기본 20개(1–200), `truncated`는 추가 결과 표시 |
| `inspect <file> <node> [--depth N]` | 인스턴스를 펼친 간결한 레이아웃/텍스트 요약. 기본 깊이 1(0–10), 최대 200노드. 전체 데이터는 `nodes` |
| `nodes <file> <node>` | REST 형태 JSON: 펼쳐진 인스턴스와 재정의, 자동 레이아웃, 텍스트 스타일, 채우기, 효과, 연결 변수, 컴포넌트 속성, 렌더링 경계 |
| `variables <file> [node]` | 프레임에 연결된 변수. 노드 없이 실행하면 모든 로컬 변수 |
| `bundle <file> <node> <out.zip>` | 한 프레임의 디자인 번들. `{nodeId, path, warnings}` 반환 |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | 한 번 디코딩해 여러 번들을 병렬 생성. 프레임당 위 형식 또는 `{nodeId, error}`의 JSON 한 줄 |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N]` | SVG, PNG, JPG, WebP, AVIF. `--max-kb`는 손실 품질을 낮춰 크기를 맞춤. PNG/SVG는 한도 초과 시 실패. `{path, width, height, bytes, quality}` 반환 |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | 로컬 읽기 전용 뷰어. `--out`은 노드가 필요하며 독립 오프라인 스냅샷 생성 |
| `compare <file> <nodes.json> <node>` | 같은 프레임의 REST 내보내기와 차이 비교 |
| `node <file> <node>` | 원시 `.fig` 노드 하나 |
| `mcp` | stdio를 사용하는 로컬 MCP 서버 |
| `update` | 최신 릴리스로 바이너리 교체 |

`find`는 저장된 레이어를 검색해 ID, 상위 경로, 로컬 크기, 텍스트 미리보기를 반환합니다. `inspect`는 인스턴스를 펼치며 출력을 노드 200개, 텍스트당 200자로 제한합니다. `truncated`와 `textTruncated`는 생략을 표시합니다. CLI 호출은 매번 파일을 디코딩합니다. MCP 서버는 디스크의 파일이 바뀔 때까지 파싱한 파일을 호출 간에 유지합니다.

`<file>`에 `gs://bucket/object`를 지정할 수도 있습니다. `gcloud` 로그인으로 다운로드하고 캐시합니다. 사용법은 `figlens --help`를 실행하세요.

<a id="help-figlens-grow"></a>

## figlens 응원하기

figlens가 다음 디자인을 코드로 구현하는 데 도움이 됐다면 [별을 눌러](https://github.com/runkids/figlens) 다른 사람들도 찾을 수 있게 해주세요. 처리하기 어려운 디자인이 있나요? 실행한 명령어와 기대한 결과를 담아 [이슈를 열어주세요](https://github.com/runkids/figlens/issues).

개발 시 Rust는 Docker에서만 실행합니다: `docker compose run --rm dev cargo test`. `make devc`는 이 체크아웃의 소스를 `figlens`로 실행하는 셸을 열고, `make help`는 컨테이너 타깃을 표시합니다. VS Code의 Reopen in Container도 같은 서비스를 사용합니다. 프로젝트 규칙은 [AGENTS.md](AGENTS.md), 기술 주제는 [wiki](wiki/README.md)를 참고하세요.

<a id="license"></a>

## 라이선스

[MIT](LICENSE). figlens는 Figma와 제휴, 보증, 후원 관계가 없는 독립 프로젝트입니다. `.fig` 형식은 문서화되지 않았으며 바뀔 수 있습니다. figlens는 본인의 Figma 계정에서 저장한 파일을 읽습니다.
