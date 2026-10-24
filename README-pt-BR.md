<p align="center">
  <img src="assets/logo.png" alt="figlens" width="220">
</p>

<h1 align="center">Seu design do Figma, pronto para seu agente de programação.</h1>

<p align="center">
  <a href="README.md">English</a> · <a href="README-de.md">Deutsch</a> · <a href="README-es.md">Español</a> · <a href="README-fr.md">Français</a> · <a href="README-ja.md">日本語</a> · <a href="README-ko.md">한국어</a> · <a href="README-pt-BR.md">Português</a> · <a href="README-zh-CN.md">简体中文</a> · <a href="README-zh-TW.md">繁體中文</a>
</p>

<p align="center">
  Dê ao seu agente de programação o layout, os textos, as variáveis, os recursos e uma imagem de referência de qualquer tela.<br>
  Leia quantas vezes quiser, no notebook, na nuvem ou em CI.<br>
  <strong>Sem aplicativo Figma, login, seat ou limite da API na máquina que faz o trabalho.</strong>
</p>

<p align="center">
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-MIT-8b78c8" alt="MIT license"></a>
  <a href="#for-ai-agents"><img src="https://img.shields.io/badge/agents-CLI_%2B_MCP-79b9a5" alt="CLI and MCP for AI agents"></a>
</p>

<p align="center">
  <a href="#from-a-local-fig-to-working-html">Ver demonstração</a> · <a href="#quick-start">Testar seu design</a> · <a href="#for-ai-agents">Conectar seu agente</a> · <a href="#pi-extension">Usar com Pi</a> · <a href="#how-close-is-it-to-figma">Precisão</a>
</p>

<a id="why-figlens"></a>

## Por que usar figlens?

“Construa esta tela” deveria vir com o layout, os textos e os recursos da tela. Com capturas, o agente precisa estimar espaçamentos e tipografia, e você continua exportando ícones à mão. O servidor MCP do Figma lê o design real, mas a cota pertence ao time dono do arquivo, um agente pode gastar a cota do dia em um único prompt, e a máquina precisa de login no Figma.

figlens lê o design a partir de uma cópia salva. No Figma, escolha **File > Save local copy** (acesso de visualização basta, a menos que o dono tenha desativado a cópia). Dê esse `.fig` ao seu agente pela CLI ou pelo servidor MCP local, e ele pode encontrar uma tela, inspecionar layout e texto e extrair os recursos necessários. Ou reúna dados, recursos e renderização de referência de um frame em um zip que qualquer agente ou colega possa usar.

| Se você já passou por isso | Com figlens, você pode |
|---|---|
| “Rate limit exceeded, try again tomorrow”, mesmo com seat pago | Ler o arquivo salvo quantas vezes precisar, sem chamadas à API do Figma, tokens ou seats |
| Seu agente roda na nuvem ou em CI | Ler o design com uma CLI nativa, sem aplicativo Figma ou sessão de navegador |
| Você fica explicando medidas e exportando recursos | Fornecer layout estruturado, tipografia, instâncias expandidas, variáveis vinculadas e recursos originais |
| Outro agente ou colega precisa da mesma tela | Compartilhar um bundle com dados, recursos, renderização de referência e instruções |

Designers continuam no Figma. Seu agente usa a stack do projeto. figlens fornece o contexto do design; o agente de programação escreve a implementação.

<details>
<summary>O formato .fig não é documentado. O que acontece quando o Figma mudar?</summary>

figlens não adivinha o formato. Cada versão é verificada contra as exportações JSON, PNG e SVG do próprio Figma dos mesmos arquivos ([resultados](#how-close-is-it-to-figma)), então uma mudança de formato aparece como regressão medida, não silenciosa. A cópia que você já salvou continua funcionando com a versão do figlens que a leu; só salvamentos novos de um Figma alterado precisam de uma atualização do figlens, que `figlens update` baixa.

</details>

<a id="from-a-local-fig-to-working-html"></a>

## De um .fig local a uma interface HTML funcional

[![Pi constrói uma interface funcional a partir de um arquivo Figma local, com prévia ao lado do terminal](assets/demo-8x.gif)](assets/demo-8x.mp4)

**[Ver o vídeo](assets/demo-8x.mp4)** · 45 segundos de time-lapse, em **8×**. Uma sessão real do Pi no Orca: ler o design local, criar HTML/CSS e testar o diálogo de compartilhamento e a área de entrada na prévia. A interface HTML usa 19 ícones SVG do bundle.

| Este exemplo | Resultado medido |
|---|---|
| Arquivo `.fig` local | 57.5 MiB |
| Frame selecionado | 153 nós, 25 recursos SVG, nenhum recurso ausente |
| Extração do bundle | **Mediana de 0.47 s** em cinco execuções com cache aquecido |

O tempo mede `figlens bundle`, **não o tempo de implementação da IA**. Medido com figlens 0.1.2 em um contêiner Docker ARM64; veja [comandos, execuções individuais e notas da gravação](wiki/accuracy.md#local-design-to-html-demo). Pi escreve o código; figlens fornece os dados. O provedor de IA ainda usa conexão de rede.

<details>
<summary>Crédito do design e limitações da demonstração</summary>

O frame “List views” vem de [MCP Apps for Claude (Community)](https://www.figma.com/community/file/1597641111449594397/mcp-apps-for-claude). A demonstração usa a fonte sans do sistema e Georgia porque as fontes originais da Anthropic não estão incluídas. Share abre um diálogo local; respostas ficam na página e são apagadas ao recarregar. A gravação usa capturas da janela real do Orca, por isso o movimento é menos fluido que uma captura de vídeo nativa.

</details>

<a id="quick-start"></a>

## Início rápido

<a id="1-open-your-design-locally"></a>

### 1. Abra seu design localmente

No Figma, escolha **File > Save local copy** e salve como `design.fig`. Depois execute:

```sh
npx @runkids/figlens preview design.fig
```

Abra a URL local exibida no terminal. Navegue pelas páginas e selecione a tela que quer construir. Você pode inspecionar espaçamentos, tipografia, variáveis e recursos ali mesmo.

Requer Node.js 18+. A primeira execução baixa, verifica e armazena em cache um binário nativo. [Instalações independentes](#installation) não precisam de Node.js, Rust ou Docker. Encerre o visualizador com Ctrl+C.

<a id="2-give-the-screen-to-your-agent"></a>

### 2. Entregue a tela ao seu agente

Baixe o bundle do frame selecionado pelo visualizador. Entregue o zip ao agente de programação com esta instrução:

> Extraia este bundle em um diretório próprio e leia context.md. Construa esta tela usando a stack e as convenções do projeto. Use os valores de layout e os recursos fornecidos e confira o resultado com references/screen.png. Informe recursos ausentes ou fontes indisponíveis.

O bundle reúne estrutura, recursos e imagem de referência, então o agente pode lê-lo com ferramentas comuns de arquivos. Para buscar e inspecionar telas diretamente, [conecte a CLI ou o servidor MCP](#for-ai-agents). [No Pi, você pode pedir a tela pelo nome](#pi-extension).

<a id="3-check-the-implementation"></a>

### 3. Confira a implementação

Importe uma captura da implementação para o visualizador. Compare lado a lado com o design ou use a sobreposição com opacidade ajustável para encontrar diferenças. A captura fica no navegador. É uma verificação visual; figlens não calcula uma nota de precisão para sua implementação.

**O `.fig` é um retrato do design naquele momento.** Salve uma nova cópia local quando o design mudar. Uma URL do Figma pode selecionar um nó dessa cópia; não baixa o design. Depois da instalação, a leitura funciona offline; seu provedor de IA e as dependências do projeto ainda podem precisar de rede.

<details>
<summary>Prefere o terminal? Encontre um frame e gere o bundle diretamente.</summary>

```sh
npx @runkids/figlens tree design.fig --depth 2
npx @runkids/figlens find design.fig "login" --type FRAME
npx @runkids/figlens inspect design.fig 12:345 --depth 1
npx @runkids/figlens bundle design.fig 12:345 frame.zip
```

Substitua `12:345` por um ID da árvore ou dos resultados da busca. Um nó aceita um ID (`12:345`), o formato de URL (`12-345`) ou uma URL do Figma com `?node-id=`. `find` busca nomes de camadas ou textos; `inspect` mostra filhos de instâncias expandidos e um resumo compacto de layout/texto. Veja [Comandos](#commands).

</details>

<a id="browse-and-hand-off-a-design"></a>

## Explorar e entregar um design

[![Inspeção local do design MCP Apps, com árvore de camadas, layout e propriedades de componentes](assets/preview-inspector.png)](assets/pi-demo.mp4)

O visualizador local permite explorar um design salvo sem uma sessão do Figma:

- Navegue por páginas e camadas, busque nomes e textos, meça distâncias e inspecione layout, tipografia, variáveis, propriedades de componentes e CSS.
- Exporte uma camada como SVG, PNG, JPG, WebP ou AVIF, ou baixe suas imagens de preenchimento originais.
- Copie seu ID, contexto estruturado ou instruções de implementação; baixe um bundle da seleção exata, incluindo subcamadas de instâncias.

Quer enviar um design que outra pessoa possa explorar? Exporte um snapshot HTML independente:

```sh
npx @runkids/figlens preview design.fig 12:345 --out preview.html
```

Ele abre sem servidor ou rede e contém dados, imagens e um bundle do escopo do snapshot. Compartilhe apenas com pessoas autorizadas a ver esses recursos. Selecionar um filho muda o contexto inspecionado; o bundle incorporado mantém o escopo original do snapshot.

O visualizador ao vivo monitora o arquivo local e atualiza após salvamentos bem-sucedidos; salvamentos parciais ou com falha marcam a visualização como desatualizada. Snapshots HTML ficam fixos. Controles, cache, automação por agentes de navegador e limitações estão no [guia de prévia](wiki/preview.md).

<a id="pi-extension"></a>

## Extensão do Pi

**Dê um `.fig` ao Pi e peça a tela.** O [pacote](https://pi.dev/packages/@runkids/figlens) inclui CLI, MCP, prévia, extensão e skill para agentes:

```sh
pi install npm:@runkids/figlens
```

Reinicie o Pi ou execute `/reload` e faça o pedido normalmente:

```text
Build the List views screen from ./design.fig using this project's components.
Show me the result and check it against the design.
```

Pi encontra a tela, lê layout e recursos, inspeciona o projeto e implementa com a stack existente. Não é preciso saber IDs de frames nem executar `/figlens open`. Se os nomes forem ambíguos, Pi pode mostrar alguns candidatos. MCP inicia automaticamente, sem instalação global da CLI.

![Peça uma tela ao Pi: ele encontra o design, usa sua stack e confere o resultado](assets/pi-workflow.svg)

**Reutilização opcional de componentes:** peça ao Pi para salvar correspondências reutilizáveis e use `/figlens review` para aceitar ou editar propostas pendentes do projeto. As decisões ficam em `.figlens/component-mappings.md`; variantes ou código registrado que mudarem precisam de revisão. Não é necessário ter correspondências para construir uma tela.

[![Inspecionar o design local, revisar um componente HTML existente no Pi e reutilizá-lo no próximo frame](assets/pi-demo.gif)](assets/pi-demo.mp4)

**[Ver o fluxo opcional de correspondências](assets/pi-demo.mp4)** · Gravação editada de 20 segundos da v0.3.0: inspecionar MCP Apps, aprovar uma correspondência preparada com HTML existente e reutilizá-la em nova sessão. A geração do modelo não é exibida nem cronometrada. [Guia, comandos e limitações](pi/README.md) · [Notas de gravação e verificação](wiki/pi.md#demo-and-verification).

Requer Pi 1.3.0+. A extensão npm usa o lançador CLI incluído; o primeiro início, incluindo o início automático do MCP, baixa e verifica o binário nativo correspondente. Homebrew e arquivos de release fornecem CLI nativa e visualizador. Para desenvolvimento, veja a [configuração a partir do código-fonte](pi/README.md#install-once).

<a id="the-design-bundle"></a>

## O bundle de design

Um frame, um zip. Estrutura para construir, recursos para usar e uma imagem para comparar.

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

Precisa de apenas uma parte? Exporte JSON ou renderize um nó diretamente:

```sh
npx @runkids/figlens nodes design.fig 12:345 > frame.json
npx @runkids/figlens variables design.fig 12:345 > variables.json
npx @runkids/figlens export design.fig 12:345 icon.svg
npx @runkids/figlens export design.fig 12:345 hero.webp --scale 2 --quality 80
```

A exportação SVG suporta frames, grupos, ícones e subcamadas exatas de instâncias: cada subárvore selecionada vira um SVG independente com imagens originais incorporadas e texto em contornos de glifos. Não precisa extrair um bundle. SVG mantém as dimensões originais; `--scale` e `--quality` só se aplicam a formatos raster. Via MCP, exportações raster são reduzidas para caber em `maxSize` (1600 px no lado maior por padrão; 0 desativa), para que as imagens inline fiquem pequenas.

<a id="how-close-is-it-to-figma"></a>

## Qual é a fidelidade ao Figma?

figlens é verificado com exportações JSON, PNG e SVG do próprio Figma. Os resultados registrados em **826 frames de produção** e **cinco arquivos Community** incluem:

| Verificação | Resultado |
|---|---|
| Estrutura, nomes, visibilidade, limites e texto dos nós | Idênticos em 771 / 826 frames de produção |
| Imagens de preenchimento originais | 2,608 / 2,608 idênticas byte a byte |
| Recursos SVG | 5,014 / 5,197 dentro do limiar de diferença de pixels |
| Bundles de frames Community | 306 / 306 sem recursos ausentes |

São medições desses arquivos, sem garantia para todo design. Veja o [relatório de precisão](wiki/accuracy.md) para datas, limiares, resultados de renderização e método.

**Limitações conhecidas:** constraints, layoutAlign/layoutGrow, grades de layout e definições de propriedades de componentes ainda não são emitidos. A renderização ignora desfoque de fundo e alguns ajustes de imagem (contraste, temperatura, matiz, realces e sombras), e desenha gradientes angulares e de diamante como radiais. O texto usa contornos de glifos salvos e dispensa fontes na renderização, mas textos sobrescritos podem manter o último layout do Figma. Os arquivos de fontes para sua implementação não estão incluídos.

<a id="for-ai-agents"></a>

## Para agentes de IA

Use a CLI por meio de um skill para agentes ou conecte o mesmo binário como servidor MCP.

<a id="agent-skill"></a>

### Skill para agentes

O [skill do figlens](skills/figlens/SKILL.md) ensina o agente a encontrar frames, ler o bundle em partes e construir a partir dos valores fornecidos.

Para reproduzir um design com cuidado, o [fluxo de planejamento](skills/figlens/references/planning.md) relaciona o escopo solicitado ao código existente, à entrega de imagens e aos critérios de aceitação. No Pi, `/figlens plan <request>` prepara um plano antes da implementação; uma região pequena continua sendo uma tarefa pequena. Use `/figlens open [description]` para encontrar o design com base no que você sabe; o Pi pergunta pelos detalhes que faltam sem exigir IDs de nós. Use `/figlens preview [description]` para iniciar o visualizador interativo local e obter seu link; o Pi identifica o arquivo e a tela sem exigir IDs de nós.

Instale com a [Skills CLI](https://github.com/vercel-labs/skills):

```sh
npx skills add runkids/figlens --skill figlens
```

Para gerenciar e atualizar skills no Claude Code, Codex, Cursor, Gemini CLI e outras ferramentas, use [skillshare](https://github.com/runkids/skillshare):

```sh
skillshare install github.com/runkids/figlens/skills/figlens
skillshare sync
```

`skillshare update --all` mantém tudo atualizado. Você também pode copiar `skills/figlens/` para o diretório de skills do agente (`~/.claude/skills/` no Claude Code). Os arquivos de release também o incluem.

<a id="mcp-server"></a>

### Servidor MCP

Gerencie servidores MCP e skills com [skillshare](https://github.com/runkids/skillshare). Defina figlens uma vez e sincronize a conexão com Claude Code, Codex e Cursor:

```sh
skillshare mcp add figlens --global \
  --target claude --target codex --target cursor --no-tui -- npx -y @runkids/figlens mcp
skillshare sync mcp --global
```

Veja o [guia MCP do Skillshare](https://skillshare.runkids.cc/docs/reference/commands/mcp/) para outros clientes e importação de configurações existentes.

Configuração direta do Claude Code:

```sh
claude mcp add figlens -- npx -y @runkids/figlens mcp
```

Outros clientes MCP:

```json
{ "mcpServers": { "figlens": { "command": "npx", "args": ["-y", "@runkids/figlens", "mcp"] } } }
```

Com uma instalação independente de `figlens`, use:

```json
{ "mcpServers": { "figlens": { "command": "figlens", "args": ["mcp"] } } }
```

Para manter o servidor dentro de pastas escolhidas, adicione `--root`: `"args": ["mcp", "--root", "/caminho/para/designs"]`. Todo `file` e caminho de saída que uma chamada nomear deve ficar sob uma das raízes; repita `--root` para mais pastas. Chamadas cujos argumentos saem do esquema da ferramenta também são recusadas, pois as conversões legadas poderiam transformar qualquer um deles em caminho.

O servidor roda localmente por stdio e expõe `tree`, `find`, `inspect`, `nodes`, `variables`, `bundle` e `export`. Use caminhos absolutos: caminhos relativos partem do diretório onde o cliente iniciou figlens.

Comece com `find` e `inspect` para selecionar um frame. `export` salva o arquivo e retorna metadados. Formatos raster também retornam conteúdo de imagem MCP, para o agente ver o design sem abrir um arquivo local; SVG retorna apenas metadados. PNG tem o suporte mais amplo entre clientes; reduza `scale` em frames grandes. `find`, `inspect`, `bundle` e `export` também retornam JSON estruturado.

<a id="installation"></a>

## Instalação

Use `npx @runkids/figlens` com Node.js 18+ ou instale a CLI independente:

| Plataforma | Comando |
|---|---|
| macOS / Linux | `curl -fsSL https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.sh \| sh` |
| Homebrew | `brew install runkids/tap/figlens` |
| Windows (PowerShell) | `irm https://raw.githubusercontent.com/runkids/figlens/main/scripts/install.ps1 \| iex` |

Os binários cobrem macOS (Apple silicon, Intel), Linux (arm64, x86_64, musl estático) e Windows (x86_64; ARM64 por emulação). Atualize instalações independentes com `figlens update`.

<a id="commands"></a>

## Comandos

| Comando | Saída |
|---|---|
| `tree <file> [node] [--depth N] [--json]` | IDs, tipos e nomes dos nós; texto indentado (menos contexto para o agente) ou JSON aninhado com `--json` |
| `find <file> <query> [--type TYPE] [--node ID] [--limit N]` | Busca de substrings em nomes e textos sem diferenciar maiúsculas; caminhos, tamanhos e prévias. Padrão de 20 resultados (1–200); `truncated` indica mais |
| `inspect <file> <node> [--depth N] [--images]` | Resumo compacto de layout/texto com instâncias expandidas. Profundidade padrão 1 (0–10), até 200 nós; use `nodes` para todos os dados. `--images` inclui pixels e bytes da origem e o recorte por uso; a contagem de pixels não diagnostica desfoque. |
| `nodes <file> <node>` | JSON no formato REST: instâncias e sobrescritas expandidas, auto-layout, estilos de texto, preenchimentos, efeitos, variáveis vinculadas, propriedades e limites de renderização |
| `variables <file> [node]` | Variáveis vinculadas a um frame; sem nó, todas as variáveis locais |
| `bundle <file> <node> <out.zip>` | Bundle de design de um frame; retorna `{nodeId, path, warnings}` |
| `bundle <file> --frames <list> --out-dir <dir> [--jobs N]` | Vários bundles em paralelo com uma decodificação; uma linha JSON por frame, como acima ou `{nodeId, error}` |
| `export <file> <node> <out> [--scale N] [--quality 1-100] [--max-kb N] [--max-size N]` | SVG, PNG, JPG, WebP ou AVIF; `--max-kb` reduz a qualidade com perda até caber; PNG/SVG falham acima do limite; retorna `{path, width, height, bytes, quality}` |
| `preview <file> [node] [--out preview.html] [--port N] [--bind IP]` | Visualizador local somente leitura; `--out` exige um nó e cria um snapshot offline independente |
| `compare <file> <nodes.json> <node>` | Diferenças em relação à exportação REST do mesmo frame |
| `node <file> <node>` | Um nó `.fig` bruto |
| `mcp` | Servidor MCP local por stdio |
| `update` | Substitui o binário pela release mais recente |

`find` busca camadas salvas e retorna IDs, caminhos dos ancestrais, tamanhos locais e prévias de texto. `inspect` expande instâncias e limita a saída a 200 nós e 200 caracteres por texto; `truncated` e `textTruncated` indicam omissões. Cada chamada da CLI decodifica o arquivo; o servidor MCP mantém o arquivo analisado entre chamadas até ele mudar no disco.

`<file>` também aceita `gs://bucket/object`, baixado com seu login do `gcloud` e armazenado em cache. Execute `figlens --help` para ver o uso.

<a id="help-figlens-grow"></a>

## Ajude o figlens a crescer

Se figlens ajudar a transformar seu próximo design em código, [dê uma estrela](https://github.com/runkids/figlens) para outras pessoas encontrarem o projeto. Algum design deu problema? [Abra uma issue](https://github.com/runkids/figlens/issues) com o comando executado e o resultado esperado.

No desenvolvimento, Rust roda apenas em Docker: `docker compose run --rm dev cargo test`. `make devc` abre um shell onde `figlens` executa o código deste checkout; `make help` lista os alvos do contêiner. Reopen in Container do VS Code usa o mesmo serviço. Veja as regras em [AGENTS.md](AGENTS.md) e os temas técnicos no [wiki](wiki/README.md).

<a id="license"></a>

## Licença

[MIT](LICENSE). figlens é um projeto independente, sem afiliação, endosso ou patrocínio do Figma. O formato `.fig` não é documentado e pode mudar; figlens lê arquivos salvos da sua própria conta Figma.
