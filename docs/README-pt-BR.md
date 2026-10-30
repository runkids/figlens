# figlens — Guia de uso

[← README](../README-pt-BR.md)

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

- [Instalação independente](#install)
- [Por que figlens em vez do MCP oficial do Figma?](#why)
- [O que o agent recebe](#bundle)
- [Extensão do Pi](#pi)
- [Explore um design sem o Figma](#viewer)
- [Servidor MCP e skill para agents](#mcp-and-skill)
- [Referência da CLI](#commands)

<a id="install"></a>

## Instalação independente

**Sem Node.js?** Instale o binário independente e use `"command": "figlens", "args": ["mcp"]` no lugar:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

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
| Fidelidade | Parser independente com [lacunas conhecidas](../README-pt-BR.md#limitations) | Mantido pelo Figma |

Ler um `.fig` funciona offline. Instalar e atualizar o figlens, entradas `gs://`, seu provedor de modelo e as dependências do projeto ainda precisam de rede.

<a id="bundle"></a>

## O que o agent recebe

Um frame, um zip. Estrutura para construir, assets para usar, uma imagem para conferir.

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

Precisa de uma parte só? A CLI faz isso direto:

```sh
npx -y @runkids/figlens find design.fig "login" --type FRAME     # IDs de frames por nome ou texto
npx -y @runkids/figlens inspect design.fig 12:345                 # resumo compacto de layout e texto
npx -y @runkids/figlens nodes design.fig 12:345 > frame.json      # JSON no formato REST
npx -y @runkids/figlens export design.fig 12:345 icon.svg         # SVG, PNG, JPG, WebP ou AVIF
npx -y @runkids/figlens bundle design.fig 12:345 frame.zip        # o zip acima
```

Exportações SVG incorporam os preenchimentos de imagem originais e convertem o texto em contornos de glifos, então nenhuma fonte é necessária. Via MCP, exportações raster também voltam como imagens inline, e o agent vê o design sem abrir um arquivo.

<a id="accuracy"></a>

### Qual é a proximidade com o Figma?

Medido contra as próprias exportações JSON, PNG e SVG do Figma em **826 frames de produção** e cinco arquivos Community (2026-10-22):

| Verificação | Resultado |
|---|---|
| Estrutura, nomes, visibilidade, limites e texto dos nós | Idênticos em 771 / 826 frames |
| Preenchimentos de imagem originais | 2,608 / 2,608 idênticos byte a byte |
| Assets SVG | 5,014 / 5,197 dentro do limiar de diferença de pixels |
| Bundles de frames Community | 306 / 306 sem assets ausentes |

Resultados nesses arquivos, não uma garantia para todo design. Método e limiares: [relatório de precisão](../wiki/accuracy.md).

<a id="pi"></a>

## Extensão do Pi

```sh
pi install npm:@runkids/figlens
```

Guias de instalação: [OpenCode V2](../opencode/README.md) · [OMP](../omp/README.md).

Reinicie o Pi ou execute `/reload` e peça uma tela em linguagem natural. O Pi encontra o frame, lê pelo servidor MCP registrado automaticamente e constrói com a sua stack. Sem IDs de nós, sem CLI global.

- `/figlens plan <request>` cria apenas um plano; não modifica a aplicação.
- `/figlens yolo <request>` planeja, constrói, confere e limpa o próprio plano temporário.
- `/figlens preview [description]` abre o viewer local.
- `/figlens review` aprova mapeamentos de componentes reutilizáveis propostos pelo agent.

![Peça uma tela ao Pi, deixe que ele encontre o design, construa com a sua stack e confira o resultado](../assets/pi-workflow.svg)

Passo a passo completo, comandos e limites: [pi/README.md](../pi/README.md).

<a id="viewer"></a>

## Explore um design sem o Figma

[![Inspecione o design MCP Apps localmente, com sua árvore de camadas, layout e propriedades de componentes](../assets/preview-inspector.png)](../assets/pi-demo.mp4)

```sh
npx -y @runkids/figlens preview design.fig                        # viewer local, Ctrl+C para parar
npx -y @runkids/figlens preview design.fig 12:345 --out preview.html   # snapshot HTML offline para enviar a qualquer pessoa
```

Navegue por páginas e camadas, busque, use a busca ranqueada e os usos exatos da seleção, meça, inspecione layout, tipografia, variáveis e CSS, exporte qualquer camada, copie um prompt de implementação ou baixe um bundle da seleção. Importe uma captura de tela da sua implementação para sobrepô-la ao design. Detalhes: [guia de preview](../wiki/preview.md).

<a id="mcp-and-skill"></a>

## Servidor MCP e skill para agents

**MCP.** O servidor roda localmente por stdio e expõe `workflow`, `tree`, `find`, `search`, `usages`, `inspect`, `nodes`, `variables`, `bundle`, `verify`, `index`, `jobs` e `export`. Use caminhos absolutos. Adicione `--root /path/to/designs` (repetível) para confinar todo arquivo e caminho de saída às pastas escolhidas. Armazenar índices fora dos diretórios padrão do figlens exige `--storage-root DIR` (repetível). Exportações raster são reduzidas a `maxSize` (padrão 1600 px) para manter as imagens inline pequenas; SVG retorna apenas metadados.

Com `workflow`, o agent pode ler os guias integrados `build`, `audit`, `images`, `plan`, `planning-details` ou `yolo` quando solicitado explicitamente. Eles compartilham as fontes do skill portátil; sua instalação separada é opcional para MCP. O agent executa o trabalho com suas ferramentas de projeto e navegador e as permissões do host. Ler um guia não executa o fluxo; seu uso depende do cliente e do comportamento do modelo.

Windows nativo com Claude Code: siga as [notas de configuração stdio](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server) e o wrapper `cmd /c` para `npx`.

**Skill.** O [skill do figlens](../skills/figlens/SKILL.md) ensina o agent a encontrar frames, ler o bundle em partes pequenas, planejar trabalho com escopo definido e verificar a UI em execução contra o design. Ele instala apenas instruções; a CLI ou o servidor MCP precisa estar disponível. Instale com a [Skills CLI](https://github.com/vercel-labs/skills) ou copie `skills/figlens/` para o diretório de skills do seu agent (Claude Code: `~/.claude/skills/`):

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

**Notas de instalação.** O launcher npm baixa o binário nativo correspondente no primeiro uso, verifica-o contra os checksums SHA-256 da release e o armazena em cache. Instalações por script são atualizadas com `figlens update`; as do Homebrew, com `brew upgrade figlens`. Os binários cobrem macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) e Windows (x86_64).

<a id="local-index"></a>

## Busca comum e o índice local opcional

|  | Comum (padrão) | Avançado: índice local (opcional) |
|---|---|---|
| O que faz | Lê o `.fig` a cada chamada e responde `search`, `find`, `inspect` e `export`. | Cria uma vez um índice de busca de um escopo escolhido e responde `search` a partir dele, com ranking entre fontes. |
| O que é gravado | Nenhum workspace nem banco de índice (`export` grava só os arquivos que você pedir). | Um workspace local (banco SQLite) no diretório de estado do usuário, ou no `--storage` escolhido. |
| Quando usar | Perguntas pontuais, scripts, CI. | Buscas repetidas em um arquivo; vários arquivos de um projeto como um único corpus (`--workspace <dir>`); usos exatos de componentes, variáveis, estilos e imagens. |

Uma pergunta de design comum nunca cria um índice. Você ativa uma vez, de forma explícita ("crie um índice para este design"); o figlens escolhe o armazenamento e não exige SQLite nem outra configuração.

```sh
# Comum: sem estado, não cria nada
figlens search design.fig "checkout button"

# Avançado, uma vez: escolha um escopo (um id de nó, ou 0:0 para o arquivo inteiro); o build roda em primeiro plano
figlens index prepare design.fig --scope 0:0

# Busca indexada; adicione --node quando o escopo indexado não for o arquivo inteiro
figlens search --file design.fig "checkout button" --node 0:0

# Depois de editar ou reexportar o .fig: recrie só os escopos cujos bytes mudaram
figlens index refresh --file design.fig

# Depois de mover o .fig: mantenha o workspace e faça refresh (use o workspaceId impresso por prepare)
figlens index relocate <sourceId> new/design.fig --id <workspaceId>
figlens index refresh --id <workspaceId>
```

MCP: `index` prepare devolve um job; consulte `jobs` até `succeeded` e busque com o `workspaceId` da resposta.

```json
{"tool": "index", "arguments": {"action": "prepare", "file": "/abs/path/design.fig", "scope": "0:0"}}
{"tool": "jobs", "arguments": {"action": "status", "jobId": "<jobId from the reply>"}}
{"tool": "search", "arguments": {"workspaceId": "<workspaceId from the reply>", "node": "0:0", "query": "checkout button"}}
```

Ciclo de vida: `refresh` recria só os escopos cujos bytes do `.fig` mudaram; `rebuild` recria mesmo sem mudança; `clear_cache` apaga só dados que um build pode recriar e mantém o registro e os índices publicados; `reset` apaga o banco de dados deste workspace (nunca o `.fig` nem as exportações) e só roda a pedido explícito. Novas fontes, um escopo maior, reset e recriação exigem, cada um, um pedido explícito próprio.

Moveu o `.fig`? `index relocate` mantém `workspaceId`, `sourceId` e armazenamento; depois faça `refresh` dos escopos afetados. Não há descoberta automática nem fusão por hash, e não é preciso reset.

Dados: o banco SQLite guarda caminhos locais registrados, nomes de camadas e texto efetivo dos escopos escolhidos, postings de busca, relações exatas (contenção, instâncias, variantes, vínculos de variáveis/estilos/imagens, destinos de protótipo) e as definições a que apontam, hashes, gerações e jobs. O `.fig` original permanece inalterado onde está; seus bytes brutos não são guardados no banco.

Pi, OpenCode e OMP usam o mesmo índice por meio de sua própria entrada opt-in, e o workspace é reutilizado entre reinícios e hosts. Não implementado: monitoramento de arquivos, ferramentas de usages no Pi, OpenCode e OMP, vetores, eventos e assinaturas.

<a id="commands"></a>

## Referência da CLI

| Comando | Saída |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | IDs, tipos REST (como em `nodes`) e nomes dos nós; texto indentado, ou JSON aninhado com o tamanho de cada nó, se está oculto e o modo de auto layout, mais `firstText` (primeiro texto visível) nos nós de uma página ou seção |
| `tree <file> [node] --top-frames [--json]` | Só as telas, uma por linha, prontas para `--frames -`: frames, componentes, conjuntos de componentes e instâncias de uma página ou de suas seções (grupos e páginas internas ignorados); `--json` acrescenta os ids de página e seção e o primeiro texto visível de cada tela (`firstText`), que separa os rótulos do canvas das telas |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Busca sem diferenciar maiúsculas em nomes de camadas e textos; caminhos, tamanhos e prévias. Padrão de 20 resultados (1-200) |
| `search <file> <query> [--node ID] [--type T[,T]] [--limit N] [--cursor C] [--include-hidden] [--include-internal] [--mode text]` | Busca ranqueada em nomes de camadas e texto completo com overrides de instância, propriedades de texto/booleanas e swaps aplicados; referências exatas de nós (incluindo ids de subcamadas `I…;…` para `inspect`), motivos de correspondência, intervalos UTF-16, cobertura e páginas por cursor. Padrão de 20 (1-200); camadas ocultas e páginas internas exigem sua flag. Com `--file`/`--workspace`/`--id` em vez de `<file>`, lê o índice preparado do workspace (veja `index`) |
| `usages <file> <node\|resource> [--scope ID] [--relation R[,R]] [--out] [--snapshot S] [--include-hidden] [--limit N] [--cursor C]` | Relações exatas de uma camada ou recurso (`VariableID:…`, `StyleKey:…`, `image:…`) em um escopo salvo, com instâncias aninhadas e swaps: onde um componente, set, variável, estilo ou imagem é usado (padrão) ou para o que uma camada aponta (`--out`). Cada resultado mantém a referência exata, relação, estado (`resolved`, `missing`, `unresolved`, …), base e evidência; um nome nunca conta como identidade. Sem totais: a cobertura diz completa ou parcial. Padrão 50 (1-200). Com `--file`/`--workspace`/`--id` no lugar de `<file>`, lê o índice preparado de `--scope` do workspace |
| `inspect <file> <node> [--depth N] [--images] [--states] [--compare-variant ID] [--flat]` | Resumo compacto de layout/texto expandido, no máximo 200 nós. `--images` adiciona dimensões de bitmap e áreas de recorte. `--flat` imprime uma linha por nó: posição relativa ao nó, tamanho, texto, fonte, preenchimento, traço, raio, auto-layout |
| `nodes <file> <node>` | O objeto por nó de uma resposta REST de nodes (`document`, `components`, `componentSets`, `styles`): instâncias e overrides expandidos, auto-layout, estilos de texto, preenchimentos, efeitos, variáveis vinculadas, propriedades de componentes, limites de renderização |
| `nodes <file> --frames <list>`, `--frames -` (stdin), `--ids <node>...` | Uma decodificação, muitos nós: uma linha JSON por id listado, a saída de `nodes` com seu `nodeId`, ou `{nodeId, error}` |
| `variables <file> [node]` | Variáveis vinculadas a um frame; sem nó, todas as variáveis locais |
| `bundle <file> <node> <out.zip> [--states]` | Um bundle de design de um frame; imprime `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N] [--states]`, `--frames -` (stdin) | Vários bundles a partir de uma decodificação, em paralelo |
| `verify <bundle>` | Confere um bundle (`.zip` ou diretório extraído) com o manifesto: arquivos verificados, ausentes e alterados, assets ausentes, as fontes a confirmar no projeto e `findings`: cada entrada de handoff ausente, não suportada ou não verificada, com nós, evidência e próxima ação |
| `export <file> <node> <out> [--bounds layout] [--scale N] [--quality 1-100] [--max-kb N] [--max-size N] [--match-dir DIR]` | SVG, PNG, JPG, WebP ou AVIF; `--max-kb` reduz a qualidade com perda até caber; `--bounds layout` exporta a caixa de layout em vez dos limites de renderização; `--match-dir DIR` (só SVG, com subpastas) lista os SVGs que desenham os mesmos contornos (`matches`) e até 10 formas parecidas com pontuação (`similar`) |
| `export <file> --frames <list> --out-dir <dir> [--format F] [--scale N] ...`, `--frames -` (stdin) | Uma decodificação, muitas exportações (padrão `png`): grava `<dir>/<nodeId>.<format>`, uma linha JSON por id, a saída de `export` com seu `nodeId`, ou `{nodeId, error}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Viewer local somente leitura; `--out` grava um snapshot offline independente |
| `compare <file> <nodes.json> <node>` | Diferenças em relação a uma exportação REST do mesmo frame |
| `node <file> <node>` | Um nó `.fig` bruto |
| `index prepare\|refresh\|rebuild\|clear\|status\|add\|remove\|relocate\|reset` | Workspace local no diretório de estado do figlens do usuário, ou em um `--storage DIR` que o workspace passa a lembrar. `prepare <file> --scope NODE` cria uma vez o índice de busca desse escopo; depois `search --file <file>` (ou `--workspace DIR`) responde sem reanalisar, com ranking conjunto das fontes do workspace. `refresh` recria os arquivos alterados, `rebuild` todos ou uma `--source`/`--scope`, `clear` remove índices sem uso; o índice anterior continua servindo até o novo ficar pronto. `reset` remove apenas o banco de dados do workspace, nunca designs, exportações ou o local lembrado |
| `jobs status\|cancel <jobId>` | Ver ou cancelar a criação de um índice; `cancel` retorna depois que a criação parou |
| `mcp` | Servidor MCP local por stdio |
| `update` | Substitui o binário pela release mais recente |

Para variantes de componentes relacionadas, use `inspect --states` ou `bundle --states` (MCP: `states: true`, false por padrão; sem alterar a configuração). As evidências preservam origem, valores desconhecidos e marcações de truncamento. Os bundles adicionam `design/states.json` e um resumo em `context.md`. Consulte o escopo e os limites em [evidências de estados](../wiki/states.md).

Compare duas variantes do mesmo component set com `inspect <file> <variantA> --compare-variant <variantB>` (MCP `compareVariant`). `variantComparison` mostra diferenças dos campos e conjuntos de valores armazenados, com IDs de origem e valores desconhecidos, sem parear nós filhos. Consulte os [limites](../wiki/states.md#compare-two-variants).

Um argumento de nó aceita `12:345`, `12-345` ou uma URL do Figma com `?node-id=`. `<file>` também pode ser `gs://bucket/object`, baixado com seu login do `gcloud` e armazenado em cache. Consulte o [guia de snapshots do GCS](../wiki/gcs.md) para a troca por um objeto fixo, as permissões necessárias e os limites de atualização. Cada chamada da CLI decodifica o arquivo; MCP reutiliza apenas o último arquivo local sem alterações. Chamadas remotas analisam o arquivo novamente. Execute `figlens --help` ou `figlens inspect --help` (também `-h`) para ver o uso completo da CLI.

`inspect` inclui redimensionamento do texto, truncamento, máximo de linhas e decoração registrados. Um pai sem auto-layout traz `childSpacing: "derived"`: os espaços entre os filhos são diferenças de coordenadas, não uma regra de auto-layout; `constraints` ainda pode fixá-los. `handoffDiagnostics` informa semântica salva, mas não emitida, com contagens, exemplos e [limites de cobertura](../wiki/semantics.md#known-handoff-gaps). As saídas protegem a origem e substituem o destino após a escrita completa. MCP limita as linhas de requisição a 1 MiB e as respostas completas a 8 MiB; exceder o limite gera um erro explícito. O JSON completo continua disponível pela CLI.
