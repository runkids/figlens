<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">코딩 에이전트가 Figma 파일을 읽습니다.<br>Figma 앱도, 로그인도, API 할당량도 필요 없습니다.</h1>

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
  <a href="#start">시작</a> · <a href="#demo">데모</a> · <a href="#why">이유</a> · <a href="#bundle">에이전트가 받는 것</a> · <a href="#pi">Pi</a> · <a href="#limitations">제한 사항</a>
</p>

**문제.** 화면을 구현하려면 에이전트에 레이아웃 값, 텍스트, 에셋이 필요합니다. 원격 호스트나 CI에서는 실행 중인 데스크톱 앱에 의존하지 않고 코드와 함께 디자인 정보에 접근할 수 있어야 합니다.

**해결.** Figma에서 **File > Save local copy**를 선택하세요. `design.fig`를 코드 옆에 두면 됩니다. figlens가 로컬에서 파싱해 레이아웃, 텍스트, 스타일, 변수, 아이콘, 이미지와 모든 frame의 참조 렌더링을 에이전트에게 제공합니다. 네이티브 바이너리 하나로 Figma에 대한 네트워크 호출은 없습니다.

<a id="start"></a>

## 시작하기

**디자인 파일이 아직 없나요?** [작은 공개 예제로 설정을 확인하세요](examples/first-success/README.md): 사용할 바이너리, frame 검색, bundle 내용을 검증합니다. 모델은 필요하지 않습니다.

프로젝트에 `design.fig`를 놓고 사용하는 에이전트를 선택하세요.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

새 세션을 시작하고 `/mcp`를 확인하세요. 원하는 화면을 평소 쓰는 말로 요청하면 됩니다. MCP에는 계획, 구현, 이미지 준비 가이드가 내장되어 있어 skill을 별도로 설치할 필요가 없습니다.

**Pi** (1.3.0+): 패키지 하나에 CLI, MCP, viewer, skill이 모두 들어 있습니다.

```sh
pi install npm:@runkids/figlens
```

**모든 MCP 클라이언트** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

[독립 바이너리 설치](docs/README-ko.md#install)

그런 다음 화면을 요청하세요.

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

에이전트는 frame을 찾고 디자인 데이터, 에셋, 참조 이미지를 bundle로 묶은 뒤 프로젝트의 기술 스택으로 화면을 구현합니다. 공통 워크플로가 폰트와 에셋 확인을 안내하며, 부족한 항목은 보고하면서 기록된 대안으로 레이아웃 구현을 계속합니다. 변경된 파일과 브라우저 비교 결과를 보여 달라고 요청하세요.

<a id="demo"></a>

## 데모: Pi가 .fig로 React 화면 구현하기

[![Pi가 로컬 .fig로 계획하고, React 화면을 구현하고, 실시간 브라우저 옆에서 확인합니다](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

GIF는 편집 영상보다 3배 빠르게 반복 재생됩니다. 클릭하면 전체 78초 영상을 볼 수 있습니다.

**[78초 영상](assets/taskflow-demo.mp4).** Pi가 `/figlens plan`을 실행해 로컬 `.fig`의 dashboard frame을 읽고, React와 Tailwind CSS로 구현한 뒤, 실행 중인 UI를 확인하고 수정합니다. 편집된 타임랩스이며 벤치마크나 pixel-perfect 보장이 아닙니다. 디자인: [Task Flow by Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [녹화 설명](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

**[.fig에서 편집 가능한 HTML/CSS로](examples/task-list/README.md):** 조건을 통제한 작업 목록 실험에서 figlens 기반 시도 6회 모두 화면의 텍스트 19곳을 실제 DOM 텍스트로 유지하고 아이콘 4개도 보존했습니다. Docker 네트워크를 비활성화한 상태에서도 CLI와 MCP 추출에 성공했습니다. 설정 절차, 측정값, 독립 이미지 비교는 사례 페이지에서 확인할 수 있습니다.

<a id="why"></a>

## 공식 Figma MCP 대신 figlens를 쓰는 이유

디자인을 파일로 전달해야 할 때는 **figlens**를 사용하세요. 원격 에이전트, CI, 오프라인 작업, 에이전트 간 전달, 제한 없는 반복 읽기가 해당합니다. 내보내기 없이 에이전트가 실시간 디자인을 봐야 할 때는 **공식 Figma MCP**를 사용하세요.

`.fig`를 읽는 일은 오프라인입니다. figlens 설치와 업데이트, `gs://` 입력, 모델 제공업체, 프로젝트 의존성에는 여전히 네트워크가 필요합니다.

[전체 비교](docs/README-ko.md#why)

<a id="bundle"></a>

## 에이전트가 받는 것

frame 하나, zip 하나. 구현할 구조, 사용할 에셋, 비교할 이미지가 담겨 있습니다.

[Bundle 내용과 사용 예시](docs/README-ko.md#bundle) · [Figma와 얼마나 가깝나요?](docs/README-ko.md#accuracy)

## 문서

- <a id="pi"></a>[Pi 확장](docs/README-ko.md#pi)
- <a id="viewer"></a>[Figma 없이 디자인 둘러보기](docs/README-ko.md#viewer)
- <a id="mcp-and-skill"></a>[MCP 서버와 에이전트 skill](docs/README-ko.md#mcp-and-skill)
- <a id="commands"></a>[CLI 레퍼런스](docs/README-ko.md#commands)

<a id="limitations"></a>

## 제한 사항

- **스냅샷이며 실시간 동기화가 아닙니다.** 디자인이 바뀌면 새 `.fig`를 내보내세요. figlens는 Figma URL에서 디자인을 다운로드할 수 없습니다.
- **문서화되지 않은 형식.** figlens는 파일에 내장된 스키마를 디코딩하므로 새 Figma 내보내기에는 figlens 업데이트가 필요할 수 있습니다.
- **폰트와 텍스트:** 폰트 파일은 포함되지 않으며, 재정의된 텍스트에 Figma의 저장된 레이아웃이 남을 수 있습니다. 사용 허가를 받은 프로젝트 폰트를 사용하고 결과를 비교하세요.
- **지원 범위:** [렌더링 개선](https://github.com/runkids/figlens/issues/36)과 [핸드오프 의미 정보](https://github.com/runkids/figlens/issues/37)는 이슈로 추적합니다. [현재 의미 정보 지원 범위](wiki/semantics.md)도 참고하세요.
- **출력을 확인하세요.** bundle의 `warnings`, `missing-assets.json`, `context.md`의 해결되지 않은 변수를 읽고, 구현 결과를 디자인과 비교하세요.
- 알려진 가시적 렌더링 제한은 노드의 `_figlens.renderDiagnostics`, 개수가 제한된 `inspect` 요약, bundle `warnings`에 표시됩니다. `missing-assets.json`은 에셋 실패만 기록합니다. 경고가 없어도 픽셀 일치를 보장하지 않습니다.

## figlens 응원하기

figlens가 디자인을 코드로 구현하는 데 도움이 됐다면 [별을 눌러](https://github.com/runkids/figlens) 다른 사람들도 찾을 수 있게 해주세요. 읽지 못하는 파일을 만났나요? 실행한 명령어와 기대한 결과를 담아 [이슈를 열어주세요](https://github.com/runkids/figlens/issues).

개발 시 Rust는 Docker에서만 실행합니다: `docker compose run --rm dev cargo test`. [AGENTS.md](AGENTS.md)와 [wiki](wiki/README.md)를 참고하세요.

## 라이선스

[MIT](LICENSE). figlens는 Figma와 제휴, 보증, 후원 관계가 없는 독립 프로젝트입니다. `.fig` 형식은 문서화되지 않았으며 바뀔 수 있습니다. 접근하고 공유할 권한이 있는 파일과 에셋만 사용하세요.
