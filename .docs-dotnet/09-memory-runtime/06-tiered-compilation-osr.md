# Tiered compilation (tier0/tier1) e OSR (On-Stack Replacement)

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Valores numéricos abaixo vêm de `src/coreclr/inc/clrconfigvalues.h` (build release) e
> podem mudar entre versões; o documento de design de 2018 cita valores históricos.

## Mecanismo no .NET

### Ideia geral

Segundo `docs/design/features/tiered-compilation.md` (Noah Falk, 2018), tiered compilation
"permite ao runtime substituir diferentes implementações em código de máquina do mesmo método
durante a vida da aplicação" e serve a dois objetivos:

- **Startup**: primeiro gerar código de baixa qualidade e barato; trocar depois por versão
  melhor se o método ficar quente. A economia de tempo de JIT "tipicamente domina" o custo de
  rodar código ruim por pouco tempo.
- **Steady-state**: código vindo de imagens **ReadyToRun** (pré-compilado) que fica quente é
  substituído por código JIT, normalmente melhor (o JIT vê as dependências carregadas e o
  suporte da CPU).

### Os tiers (enum do runtime)

`TieredCompilationManager::GetJitFlags` (`src/coreclr/vm/tieredcompilation.cpp:1027`) enumera:

| Tier | Flags enviadas ao JIT | Significado |
|---|---|---|
| `OptimizationTier0` | `CORJIT_FLAG_TIER0` | código rápido de gerar, pouco otimizado; inclui patchpoints de OSR |
| `OptimizationTier0Instrumented` | `BBINSTR` + `TIER0` | Tier0 com instrumentação para PGO |
| `OptimizationTier1` | `TIER1` | código totalmente otimizado |
| `OptimizationTier1Instrumented` | `BBINSTR` + `TIER1` | otimizado com instrumentação (partindo de R2R) |
| `OptimizationTier1OSR` | `OSR` + `TIER1` | versão otimizada específica para OSR |
| `OptimizationTierOptimized` | (nenhuma tier flag) | método não elegível a tiering: compila otimizado direto |

Código **ReadyToRun** é tratado como o "tier0" daquele método: "For methods that are
precompiled in ReadyToRun images, the precompiled code is the tier0 version"
(`tiered-compilation.md`).

Quais métodos participam: `MethodDesc::IsEligibleForTieredCompilation` (`method.hpp`); métodos
marcados com `AggressiveOptimization` vão direto a Tier1 sem perfil (diagrama em
`docs/design/features/DynamicPgo-InstrumentedTiers.md`).

### Política de promoção

`tiered-compilation.md` descreve (política da 2.1) dois mecanismos que precisam ser satisfeitos
para promover Tier0 -> Tier1:

1. **Contagem de chamadas**: o método precisa ser chamado >= 30 vezes (medido por um contador
   de chamadas).
2. **Timer de "fim do startup"**: um timer de 100 ms é reiniciado sempre que ocorre JIT de
   Tier0; só quando expira sem novo JIT Tier0 é que a contagem de chamadas começa. Motivo:
   contadores e threads de background de Tier1 atrasavam as threads de primeiro plano durante
   o startup.

Depois disso: o método entra numa fila, é compilado em **thread de background** e a versão Tier1
é ativada.

Valores no código atual (`clrconfigvalues.h:466-495`, release):

| Config | Padrão | Descrição |
|---|---|---|
| `TieredCompilation` | 1 | habilita |
| `TC_QuickJit` | 1 | usa JIT rápido onde apropriado |
| `TC_QuickJitForLoops` | 1 (quando `FEATURE_ON_STACK_REPLACEMENT`; senão 0) | quick JIT também para métodos com loops |
| `TC_CallCountThreshold` | 30 | chamadas em Tier0 antes da promoção |
| `TC_CallCountingDelayMs` | 100 | atraso "perpétuo" aplicado ao call counting e ao JIT de tiers altos enquanto houver atividade Tier0 |
| `TC_DelaySingleProcMultiplier` | 10 | multiplicador do atraso em máquina de 1 processador |
| `TC_BackgroundWorkerTimeoutMs` | 4000 | tempo ocioso do worker antes de sair |
| `TC_AggressiveTiering` | 0 | transita pelos tiers agressivamente |
| `TC_UseCallCountingStubs` | 1 | usa stubs de contagem de chamadas |
| `TieredPGO` | 1 | instrumenta Tier0 e dá contagens ao Tier1 |

(Em build `_DEBUG` os valores são minúsculos - 2 chamadas, 1 ms - "para exercitar mais caminhos
mais cedo".)

Limitações do desenho original que o próprio doc lista ("Known Issues"): o contador não trata
bem métodos quentes por conterem loops mas chamados poucas vezes; o valor fixo do timer não
escala com o hardware; o timer não reinicia depois de expirar, então fases de "startup" tardias
(ex.: servidor que recebe a primeira requisição depois de 100 ms) rodam com Tier0 por mais
tempo.

### Mecânica: CodeVersionManager, prestub e call counting

Dependências (tiered-compilation.md): **CodeVersionManager**
(`src/coreclr/vm/codeversion.cpp`) cria/ativa versões de código de um método; o **Prestub**
fornece a implementação inicial e continua a ser usado para contar chamadas; o **ThreadPool**
fornece as threads de background e timers. A implementação está em `tieredcompilation.cpp`
e `callcounting.cpp`. Notas do doc: o contador original era "utterly naive" e o custo do prestub
por chamada é alto - mitigado pelo limite de chamadas contadas e pelo custo de ~100 mil ciclos
de um JIT; o código atual tem **stubs de contagem** (`TC_UseCallCountingStubs`).

### PGO dinâmico em camadas

`docs/design/features/DynamicPgo-InstrumentedTiers.md` (diagrama mermaid) descreve:

- Método sem R2R: JIT Tier0 (sem instrumentação, com patchpoints) -> quando quente (> 30
  chamadas) -> **Tier0Instrumented** -> quando quente de novo -> **Tier1 com perfil**.
- Método com R2R: usa o código R2R (otimizado, sem instrumentação, sem patchpoints) -> quando
  quente -> **Tier1Instrumented** (otimizado, instrumentado) -> **Tier1 com perfil**.

Motivos declarados: R2R também deve se beneficiar de PGO; a instrumentação **não** deve
atrasar o startup, e métodos que nunca avançam não devem ser instrumentados.
`TieredPGO_InstrumentOnlyHotCode` é 1 por padrão (`clrconfigvalues.h:523`). Exceção em
`GetJitFlags`: métodos Tier0 com loops elegíveis a OSR recebem `BBINSTR_IF_LOOPS` para ter
perfil caso se autopromovam via OSR.

### OSR: trocando o código de uma execução em andamento

Problema (`docs/design/features/OnStackReplacement.md`, Andy Ayers): tiering só afeta
**novas chamadas**. Um método com loop que roda uma única vez (ex.: `Main` com loop quente)
ficaria "preso" no Tier0. OSR resolve migrando um **frame ativo** do código antigo (não
otimizado) para código novo (otimizado) no meio da execução. O doc define: "OSR" = transição
**a partir de** código não otimizado; "deopt" = transição **de** código otimizado para outro
(tipicamente não otimizado) - esta última **não** é o escopo da implementação atual.

OSR é o que permite habilitar tiered compilation por padrão para quase todos os métodos
("performance-critical applications will no longer risk seeing key methods trapped in
unoptimized tier0 code").

Peças:

1. **Patchpoints**: pontos do código Tier0 onde a transição é possível. Na implementação
   atual: alvos (ou fontes, conforme estratégia) de **arestas de volta (back edges)** de loops
   na IL, em pontos onde a pilha de avaliação IL está vazia, **fora de handlers de exceção**
   e em métodos **sem `localloc`** (`src/coreclr/jit/patchpoint.cpp`: "no patchpoints in handler
   regions; no patchpoints for localloc methods"). O JIT também desabilita OSR para métodos
   com loop dentro de catch/finally (doc, seção 4.3). Estratégia configurável
   `TC_PatchpointStrategy`: 0 = fontes de back edge, 1 = alvos, **2 = adaptativa (padrão)**
   (`jitconfigvalues.h:780`).
2. **Gatilho**: **um contador local por frame**, compartilhado por todos os patchpoints,
   inicializado no prólogo com `TC_OnStackReplacement_InitialCounter` (1000). Cada patchpoint
   decrementa e, ao chegar a zero/negativo, chama um helper do runtime. Pelo `clrconfigvalues.h`
   também existem `OSR_HitLimit` (10) e `OSR_CounterBump` (1000) (internos).
3. **Método OSR**: o runtime pede ao JIT uma versão **parcial** do método (flag `OSR`),
   específica para o patchpoint (IL offset), que "incorpora o frame original": só o código
   alcançável a partir do patchpoint é compilado, com um prólogo especial de transição.
4. **Transição síncrona**: o helper compila (se necessário) e salta para o método OSR.
   O estado vivo é "todos os locais e argumentos IL + alguns valores especiais" (por isso o
   Tier0 não precisa de análise de liveness): em Tier0 nenhum estado IL fica em registradores
   através de pontos de pilha vazia, tudo está no frame.
5. **Frames e callee-saves**: o frame do OSR sobrepõe o do Tier0; em x64 o Tier0 reserva
   espaço para callee-saves e o OSR salva o restante (`OSRX64EpilogRedesign.md`).

Particularidades:

- Durante a transição **GC é proibido** (há valores vivos nos dois frames) - doc, 4.4.
- OSR é **desabilitado para código debuggable** (4.5).
- `TC_PartialCompilation` (padrão 0) permite um modo de "compilação parcial", em que o Tier0
  só compila partes prováveis (`jitconfigvalues.h:771`).
- Breakpoints em endereços nativos não migram para o método OSR (mesmo problema do tiering).

Exemplo de código (do doc): um `for` somando `i` compila em Tier0 com locais na pilha; com
patchpoints ganha um contador e um `call` ao helper no back edge.

## Por quê

- Startup importa: JIT otimizado é caro (o doc cita "100.000 ciclos por método" como ordem
  de grandeza). Gerar código simples primeiro e otimizar só o que esquenta reduz o tempo até a
  primeira resposta.
- Perfil real é melhor que adivinhar: com instrumentação, o Tier1 pode otimizar de acordo
  com o comportamento (devirtualização guardada, layout de blocos; ver arquivo 07).
- OSR cobre o buraco do tiering: sem ele, loops quentes em métodos chamados uma vez
  ficariam lentos; o time precisaria então compilar *todo* método com loop direto em Tier1,
  perdendo o benefício de startup. A configuração `TC_QuickJitForLoops` só é 1 por padrão
  porque OSR existe (`#ifdef FEATURE_ON_STACK_REPLACEMENT`).
- Princípios de OSR do doc: "pay as you go" (custo só em métodos que se beneficiam); poucas
  restrições ao codegen otimizado; testabilidade (poder forçar transições); diagnóstico
  completo.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Começar em Tier0 | Menor tempo de JIT inicial | Código lento até a promoção; sobreposição de compilações |
| Atraso de 100 ms / call counting | Evita competir com o startup | Código Tier0 roda por mais tempo; heurística frágil (doc "Known Issues") |
| Compilar em thread de background | Não bloqueia o app | Threads extras; atraso entre quente e otimizado |
| Contar chamadas via prestub/stubs | Simples | Overhead por chamada (mitigado por stubs) |
| Instrumentar para PGO | Código final melhor | Custo extra de execução e de JIT; fases extras de tier |
| OSR | Libera loops quentes de Tier0 | Complexidade enorme: patchpoints, frames, unwind, EH, GC, depuração |
| Patchpoints só em loops/pilha vazia | Metadados pequenos ("pay as you go") | Não cobre recursão ou métodos longos sem loop; proibido em handlers e `localloc` |
| Sem deopt | Mais simples | Não permite especulação agressiva com reversão (o doc discute que deopt é "um conjunto maior de desafios") |
| Múltiplos entry points em OSR | Reuso | Perda de otimizações; por isso os métodos OSR são distintos dos Tier1 |

## Como aplicar na sua linguagem

1. **Introduza tiering só depois de ter um versionamento de código funcional.** O .NET depende
   de um `CodeVersionManager` e de entry points redirecionáveis (precode/prestub, arquivo 05).
   Sem uma "indireção atualizável" não dá para trocar a versão de um método.
2. **Defina o tier inicial barato**: o mais simples é um interpretador (o próprio .NET tem um
   interpretador que usa tiered compilation para promover para código JIT, segundo o doc)
   ou um JIT sem otimizações.
3. **Heurística mínima**: contador de chamadas + atraso global de "startup". Mantenha ambos
   configuráveis e tenha em mente os problemas do doc: contagem não vê loops; timers fixos não
   escalam; fases de startup tardias.
4. **Faça a promoção assíncrona** em uma thread de compilação, com um limite de tempo por
   fatia para ser "bom cidadão" (o doc cita fatias de ~10 ms do threadpool na versão original;
   o código atual tem um worker dedicado com timeout).
5. **Se sua linguagem tem loops longos em `main`/scripts** (muito comum), você precisará de
   OSR ou de compilar métodos com loops direto otimizados. OSR é caro; uma alternativa mais
   simples é "compilar direto em tier otimizado se houver loop" (o que o .NET fazia antes de
   OSR: `TC_QuickJitForLoops=0`).
6. **Se implementar OSR, copie as restrições**: patchpoints só em pontos de pilha vazia
   (back edges), fora de handlers; estado vivo = locais + argumentos (um conjunto fixo); sem GC
   durante a transição; contador local ao frame; método OSR parcial.
7. **Projete o backend com tier como parâmetro** (flags `TIER0`, `TIER1`, `OSR`, `BBINSTR`),
   assim o mesmo compilador serve a todos os níveis.
8. **Planeje para PGO**: reserve um modo de instrumentação cuja saída (contagens por bloco,
   histogramas de tipo) o Tier1 consuma. R2R/AOT: trate o código pré-compilado como Tier0.
9. **Reserve um caminho de escape** ("AggressiveOptimization") para métodos que o autor
   sabe que devem ir direto otimizados.
10. **Ferramentas**: eventos de tiering/OSR, e a possibilidade de forçar transições em testes
    (`JitRandomOnStackReplacement`, `JitOffsetOnStackReplacement`, `OSR_LowId/HighId`).

## Referências

- `docs/design/features/tiered-compilation.md` - conceitos, política 2.1, problemas conhecidos.
- `docs/design/features/OnStackReplacement.md` - design, patchpoints, gatilhos, complicações,
  protótipo e exemplo de código.
- `docs/design/features/OsrDetailsAndDebugging.md`, `OSRX64EpilogRedesign.md`.
- `docs/design/features/DynamicPgo.md`, `DynamicPgo-InstrumentedTiers.md`, `dotnet-pgo.md`.
- `src/coreclr/vm/tieredcompilation.cpp` - `GetJitFlags`, `GetInitialOptimizationTier`,
  worker de background.
- `src/coreclr/vm/callcounting.cpp`, `codeversion.cpp`, `method.hpp`
  (`IsEligibleForTieredCompilation`).
- `src/coreclr/inc/clrconfigvalues.h` (linhas ~455-530) - configurações `TC_*`, `OSR_*`, `TieredPGO*`.
- `src/coreclr/jit/patchpoint.cpp`, `src/coreclr/jit/jitconfigvalues.h` (linhas ~766-800) -
  expansão de patchpoints e estratégias.
- `src/coreclr/inc/corjitflags.h` - flags `TIER0`, `TIER1`, `OSR`, `BBINSTR`.
