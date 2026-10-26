<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Seu coding agent lê arquivos do Figma.<br>Sem app do Figma. Sem login. Sem cota de API.</h1>

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
  <a href="#start">Começar</a> · <a href="#demo">Demo</a> · <a href="#why">Por quê</a> · <a href="#bundle">O que o agente recebe</a> · <a href="#pi">Pi</a> · <a href="#limitations">Limitações</a>
</p>

**O problema.** Seu agente precisa de valores de layout, texto e recursos para implementar uma tela. Em um servidor remoto ou no CI, esse contexto deve estar disponível ao lado do código, sem depender de um aplicativo desktop em execução.

**A solução.** No Figma, **File > Save local copy**. Coloque `design.fig` ao lado do seu código. O figlens o interpreta localmente e entrega ao seu agent layout, texto, estilos, variáveis, ícones, imagens e uma renderização de referência de qualquer frame. Um binário nativo, zero chamadas de rede ao Figma.

<a id="start"></a>

## Primeiros passos

**Ainda não tem um arquivo de design?** [Confira seu ambiente com o pequeno exemplo público](examples/first-success/README.md): verifique o binário escolhido, encontre um frame e valide seu bundle. Não é necessário um modelo.

Coloque `design.fig` no seu projeto e escolha seu agent.

**Claude Code** (Node.js 18+, macOS/Linux):

```sh
claude mcp add --transport stdio --scope local figlens -- npx -y @runkids/figlens mcp
```

Inicie uma nova sessão e confira `/mcp`. Descreva a tela com suas próprias palavras: o MCP inclui orientações sob demanda para planejamento, implementação e preparação de imagens; não é necessário instalar o skill separadamente.

**Pi** (1.3.0+): um único pacote inclui CLI, MCP, viewer e skill.

```sh
pi install npm:@runkids/figlens
```

**Qualquer cliente MCP** (Cursor, Codex, Gemini CLI, ...):

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

**Sem Node.js?** Instale o binário independente e use `"command": "figlens"` no lugar:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Depois peça uma tela:

```text
Build the task board from ./design.fig in this project.
Find the matching frame, use this project's components and image workflow,
then compare the running UI with the design and fix confirmed differences.
Report missing assets and any checks you could not run.
```

O agent encontra o frame, monta um bundle com dados do design, assets e uma imagem de referência, e implementa a tela com a sua stack. Peça que ele mostre os arquivos alterados e a comparação; um bundle sozinho não prova que a UI está certa.

<a id="demo"></a>

## Demo: o Pi cria uma tela React a partir de um .fig

[![O Pi planeja a partir de um .fig local, cria uma tela React e a confere ao lado de um navegador ao vivo](assets/taskflow-demo.gif)](assets/taskflow-demo.mp4)

O GIF em loop reproduz o vídeo editado a três vezes a velocidade. Clique para ver o vídeo completo de 78 segundos.

**[Vídeo de 78 segundos](assets/taskflow-demo.mp4).** O Pi executa `/figlens plan`, lê um frame de dashboard de um `.fig` local, o constrói com React e Tailwind CSS e depois confere e corrige a UI em execução. Time-lapse editado, não é um benchmark nem uma promessa de pixel-perfect. Design: [Task Flow por Eymen Eid](https://www.figma.com/community/file/1224268866120711793) · [notas da gravação](wiki/accuracy.md#taskflow-react-and-tailwind-demo).

<a id="why"></a>

## Por que figlens em vez do MCP oficial do Figma?

Use o **figlens** quando o design precisa viajar como arquivo: agents remotos, CI, trabalho offline, handoff entre agents, leituras repetidas ilimitadas. Use o **MCP oficial do Figma** quando o agent precisa ver o design ao vivo, sem etapa de exportação.

| | figlens | MCP oficial do Figma |
|---|---|---|
| Fonte | Snapshot `.fig` exportado | Arquivos Figma ao vivo |
| Requer | O arquivo | Login no Figma; o servidor desktop exige o app |
| Rate limits | Nenhum, parsing local | Limites do serviço do Figma |
| Agent remoto / CI | Copie o arquivo, execute o binário | Servidor remoto com login |
| Mudanças no design | Exporte um novo `.fig` | Automático |
| Handoff | Compartilhe o `.fig` ou um bundle de frame | Compartilhe o acesso ao Figma |
| Fidelidade | Parser independente com [lacunas conhecidas](#limitations) | Mantido pelo Figma |

Ler um `.fig` funciona offline. Instalar e atualizar o figlens, entradas `gs://`, seu provedor de modelo e as dependências do projeto ainda precisam de rede.

<a id="bundle"></a>

## O que o agent recebe

Um frame, um zip. Estrutura para construir, assets para usar, uma imagem para conferir.

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

Precisa de uma parte só? A CLI faz isso direto:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # IDs de frames por nome ou texto
npx -y @runkids/figlens inspect design.fig 12:345                 # resumo compacto de layout e texto
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON no formato REST
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP ou AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # o zip acima
```

Exportações SVG incorporam os preenchimentos de imagem originais e convertem o texto em contornos de glifos, então nenhuma fonte é necessária. Via MCP, exportações raster também voltam como imagens inline, e o agent vê o design sem abrir um arquivo.

### Qual é a proximidade com o Figma?

Medido contra as próprias exportações JSON, PNG e SVG do Figma em **826 frames de produção** e cinco arquivos Community (2026-10-22):

| Verificação | Resultado |
|---|---|
| Estrutura, nomes, visibilidade, limites e texto dos nós | Idênticos em 771 / 826 frames |
| Preenchimentos de imagem originais | 2,608 / 2,608 idênticos byte a byte |
| Assets SVG | 5,014 / 5,197 dentro do limiar de diferença de pixels |
| Bundles de frames Community | 306 / 306 sem assets ausentes |

Resultados nesses arquivos, não uma garantia para todo design. Método e limiares: [relatório de precisão](wiki/accuracy.md).

<a id="pi"></a>

## Extensão do Pi

```sh
pi install npm:@runkids/figlens
```

Reinicie o Pi ou execute `/reload` e peça uma tela em linguagem natural. O Pi encontra o frame, lê pelo servidor MCP registrado automaticamente e constrói com a sua stack. Sem IDs de nós, sem CLI global.

- `/figlens plan <request>` cria apenas um plano; não modifica a aplicação.
- `/figlens yolo <request>` planeja, constrói, confere e limpa o próprio plano temporário.
- `/figlens preview [description]` abre o viewer local.
- `/figlens review` aprova mapeamentos de componentes reutilizáveis propostos pelo agent.

![Peça uma tela ao Pi, deixe que ele encontre o design, construa com a sua stack e confira o resultado](assets/pi-workflow.svg)

Passo a passo completo, comandos e limites: [pi/README.md](pi/README.md).

<a id="viewer"></a>

## Explore um design sem o Figma

[![Inspecione o design MCP Apps localmente, com sua árvore de camadas, layout e propriedades de componentes](assets/preview-inspector.png)](assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C para parar
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML offline para enviar a qualquer pessoa
```

Navegue por páginas e camadas, busque, meça, inspecione layout, tipografia, variáveis e CSS, exporte qualquer camada, copie um prompt de implementação ou baixe um bundle da seleção. Importe uma captura de tela da sua implementação para sobrepô-la ao design. Detalhes: [guia de preview](wiki/preview.md).

<a id="mcp-and-skill"></a>

## Servidor MCP e skill para agents

**MCP.** O servidor roda localmente por stdio e expõe `workflow`, `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` e `export`. Use caminhos absolutos. Adicione `--root /path/to/designs` (repetível) para confinar todo arquivo e caminho de saída às pastas escolhidas. Exportações raster são reduzidas a `maxSize` (padrão 1600 px) para manter as imagens inline pequenas; SVG retorna apenas metadados.

Com `workflow`, o agent pode ler os guias integrados `build`, `images`, `plan`, `planning-details` ou `yolo` quando solicitado explicitamente. Eles compartilham as fontes do skill portátil; sua instalação separada é opcional para MCP. O agent executa o trabalho com suas ferramentas de projeto e navegador e as permissões do host. Ler um guia não executa o fluxo; seu uso depende do cliente e do comportamento do modelo.

Windows nativo com Claude Code: siga as [notas de configuração stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) e o wrapper `cmd /c` para `npx`.

**Skill.** O [skill do figlens](skills/figlens/SKILL.md) ensina o agent a encontrar frames, ler o bundle em partes pequenas, planejar trabalho com escopo definido e verificar a UI em execução contra o design. Ele instala apenas instruções; a CLI ou o servidor MCP precisa estar disponível. Instale com a [Skills CLI](https://github.com/vercel-labs/skills) ou copie `skills/figlens/` para o diretório de skills do seu agent (Claude Code: `~/.claude/skills/`):

```sh
npm install -g @runkids/figlens
npx skills add runkids/figlens --skill figlens
```

**skillshare.** Gerencie o skill e a conexão MCP no Claude Code, Codex e Cursor em um só lugar com o [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare mcp add figlens --global --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync && skillshare sync mcp --global
```

**Notas de instalação.** O launcher npm baixa o binário nativo correspondente no primeiro uso, verifica-o contra os checksums SHA-256 da release e o armazena em cache. Instalações independentes são atualizadas com `figlens update`. Os binários cobrem macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) e Windows (x86_64).

<a id="commands"></a>

## Referência da CLI

| Comando | Saída |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | IDs, tipos e nomes dos nós; texto indentado ou JSON aninhado |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Busca sem diferenciar maiúsculas em nomes de camadas e textos; caminhos, tamanhos e prévias. Padrão de 20 resultados (1-200) |
| `inspect <file> <node> [--depth N] [--images] [--states]` | Resumo compacto de layout/texto expandido, no máximo 200 nós. `--images` adiciona dimensões de bitmap e áreas de recorte |
| `nodes <file> <node>` | JSON no formato REST: instâncias e overrides expandidos, auto-layout, estilos de texto, preenchimentos, efeitos, variáveis vinculadas, propriedades de componentes, limites de renderização |
| `variables <file> [node]` | Variáveis vinculadas a um frame; sem nó, todas as variáveis locais |
| `bundle <file> <node> <out.zip> [--states]` | Um bundle de design de um frame; imprime `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]` | Vários bundles a partir de uma decodificação, em paralelo |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP ou AVIF; `--max-kb` reduz a qualidade com perda até caber |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local somente leitura; `--out` grava um snapshot offline independente |
| `compare <file> <nodes.json> <node>` | Diferenças em relação a uma exportação REST do mesmo frame |
| `node <file> <node>` | Um nó `.fig` bruto |
| `mcp` | Servidor MCP local por stdio |
| `update` | Substitui o binário pela release mais recente |

Para variantes de componentes relacionadas, use `inspect --states` ou `bundle --states` (MCP: `states: true`, false por padrão; sem alterar a configuração). As evidências preservam origem, valores desconhecidos e marcações de truncamento. Os bundles adicionam `design/states.json` e um resumo em `context.md`. Consulte o escopo e os limites em [evidências de estados](wiki/states.md).

Um argumento de nó aceita `12:345`, `12-345` ou uma URL do Figma com `?node-id=`. `<file>` também pode ser `gs://bucket/object`, baixado com seu login do `gcloud` e armazenado em cache. Consulte o [guia de snapshots do GCS](wiki/gcs.md) para a troca por um objeto fixo, as permissões necessárias e os limites de atualização. Cada chamada da CLI decodifica o arquivo; MCP reutiliza apenas o último arquivo local sem alterações. Chamadas remotas analisam o arquivo novamente. Execute `figlens --help` para ver o uso.

<a id="limitations"></a>

## Limitações

- **Snapshots, não sincronização ao vivo.** Exporte um novo `.fig` quando o design mudar. O figlens não consegue baixar um design a partir de uma URL do Figma.
- **Formato não documentado.** O figlens decodifica o schema embutido no arquivo; novas exportações do Figma podem exigir uma atualização do figlens.
- **Ainda não emitidos:** anotações do Dev Mode, grades de layout.
- **Semântica parcial:** constraints, layoutAlign/layoutGrow e definições de propriedades registradas são emitidas com origem em `_figlens.semantics`. Valores ausentes não são preenchidos; opções variant e SLOT seguem sem suporte. Esses mapeamentos ainda não foram comparados com exportações Figma da mesma versão. Veja a [cobertura](wiki/semantics.md).
- **Lacunas de renderização:** desfoque de fundo e alguns ajustes de imagem são ignorados; gradientes angulares e de diamante são desenhados como radiais. Texto com override pode manter o último layout do Figma. Os arquivos de fontes não estão incluídos.
- **Confira a saída.** Leia os `warnings` do bundle, `missing-assets.json` e as variáveis não resolvidas em `context.md`, depois compare a implementação com o design.
- As limitações visíveis conhecidas aparecem em `_figlens.renderDiagnostics` dos nós, nos resumos limitados de `inspect` e nos `warnings` do bundle; `missing-assets.json` contém apenas falhas de assets. A ausência de avisos não garante fidelidade de pixels.

## Ajude o figlens a crescer

Se o figlens transformou um design em código para você, [dê uma estrela no repo](https://github.com/runkids/figlens) para que outras pessoas o encontrem. Encontrou um arquivo que ele não consegue ler? [Abra uma issue](https://github.com/runkids/figlens/issues) com o comando e o que você esperava.

O desenvolvimento roda Rust apenas em Docker: `docker compose run --rm dev cargo test`. Veja [AGENTS.md](AGENTS.md) e o [wiki](wiki/README.md).

## Licença

[MIT](LICENSE). O figlens é um projeto independente, sem afiliação, endosso ou patrocínio do Figma. O formato `.fig` não é documentado e pode mudar; use apenas arquivos e assets que você tem permissão para acessar e compartilhar.
