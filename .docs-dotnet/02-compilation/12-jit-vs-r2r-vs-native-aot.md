# Comparação: JIT vs R2R vs Native AOT (quando cada um)

## Mecanismo no .NET

O .NET oferece três estratégias de transformar IL em código nativo. Elas não são exclusivas: **JIT + R2R coexistem no mesmo processo** (R2R é "tier0 pré-compilado" e o JIT recompila o que é quente), enquanto **Native AOT é um modelo separado** (outro runtime reduzido, outro formato de saída).

| | **JIT (CoreCLR + RyuJIT)** | **ReadyToRun (crossgen2)** | **Native AOT (ILC)** |
|---|---|---|---|
| Quando gera código nativo | Em runtime, por método, na primeira chamada | Em tempo de build, mas **mantém IL**; JIT disponível como fallback | Em tempo de build, **todo o programa**; sem JIT |
| Artefato | `.dll` IL puro (arquivo 03) | `.dll` IL **+ código nativo + tabelas** (`READYTORUN_HEADER`, arquivo 05) | Executável nativo (COFF/ELF/Mach-O) autocontido |
| Runtime necessário | CoreCLR completo instalado ou empacotado | CoreCLR completo | Runtime reduzido linkado estaticamente (`src/coreclr/nativeaot/Runtime`) |
| Metadata | ECMA-335 completa | ECMA-335 completa (continua mapeada) | Formato próprio reduzido, só o necessário (`ILCompiler.MetadataTransform`) |
| Carga dinâmica (`Assembly.Load`, `Reflection.Emit`) | Sim | Sim | Não (`PlatformNotSupportedException`; `RuntimeFeature.IsDynamicCodeSupported => false`) |
| Otimizações cross-assembly | Máximas em tier1 (vê tudo que está carregado) | Só dentro da **version bubble** (+ `NonVersionable`) | Programa inteiro (mundo fechado) |
| Qualidade do pico | Alta, com tiering + PGO dinâmico (`DynamicPgo.md`), OSR | Código R2R ≈ tier0 (melhorado por rejit a tier1) | Alta e estável, sem PGO dinâmico (PGO estático `.mibc` opcional) |
| Startup | Mais lento (JIT) — mitigado por Tier0 | Rápido | Mais rápido |
| Tamanho em disco | Menor (só IL) | Maior (IL + nativo) | Pequeno (só alcançável), mas inclui runtime |
| Working set | Memória para JIT e código | Menos JIT; páginas do arquivo | Mínimo (sem JIT, sem metadata completa) |
| Código para outra plataforma | Mesmo `.dll` | Específico de plataforma/arquitetura (`READYTORUN_FLAG_PLATFORM_NEUTRAL_SOURCE` indica origem neutra) | Por RID/arquitetura |

## JIT (referência)

- **Pipeline**: IL → RyuJIT (importação para IR, otimizações, regalloc, emissão) — ver `docs/design/coreclr/jit/ryujit-overview.md`.
- **Tiered compilation** (`docs/design/features/tiered-compilation.md`): *Tier0* = código rápido de gerar (otimizações mínimas) ou, para métodos presentes em R2R, o próprio código R2R; *Tier1* = otimizado. Promoção após N chamadas e período sem Tier0-jitting. Em `src/coreclr/inc/clrconfigvalues.h`: `TC_CallCountThreshold` (30 em builds Release; 2 em builds `_DEBUG`, "to exercise more paths sooner"), `TC_CallCountingDelayMs` (100 ms), `TC_BackgroundWorkerTimeoutMs` (4000 ms); `TieredCompilation` (habilitado por padrão), `TC_QuickJitForLoops`.
- **OSR** (`docs/design/features/OnStackReplacement.md`): troca de código no meio de um método em execução (loops longos em Tier0).
- **Dynamic PGO** (`docs/design/features/DynamicPgo.md`, `DynamicPgo-InstrumentedTiers.md`): instrumenta em tier intermediário e usa o perfil para devirtualização guardada (`jit/GuardedDevirtualization.md`), inlining agressivo, layout de blocos, etc. É a grande vantagem do JIT: "in a jitted environment profile based optimization can be done automatically".
- **Code versioning** (`code-versioning.md`): suporte a múltiplas versões de um método (tiering, rejit, profiler).
- **Vantagem de ver o hardware**: o JIT sabe o conjunto de instruções real da CPU e as dependências carregadas (doc de tiered: "At runtime the JIT is able to observe the exact dependencies that are loaded as well as CPU instruction support").

## R2R (referência)

- Cache de código resiliente a versão; as regras de *version bubble*, fixups e restrições estão em `readytorun-overview.md` e `readytorun-format.md` (arquivo 05).
- O compilador (arquivo 06) escolhe conjunto de instruções alvo (`--instruction-set`) — por padrão conservador. Instruções além do conjunto-alvo ficam a cargo do JIT em runtime (o doc de tiered compilation destaca que o JIT vê o suporte real da CPU).
- Recompilação de métodos quentes a tier1 pelo JIT corrige o "preço de versionamento" (sem inlining cruzado, indireções).

## Native AOT (referência)

- Arquitetura em `ilc-architecture.md` e limitações em arquivo 07. Sem JIT; **tudo** que pode executar deve ser conhecido em build (ou coberto por `rd.xml`/análise de reflexão).
- Para plataformas sem geração de código dinâmica, R2R é usado em **modo "sem JIT"**: o crossgen2 define `GetTargetAllowsRuntimeCodeGeneration` falso para iOS/tvOS/MacCatalyst/Browser/Wasi/wasm32 e gera `EnableCachedInterfaceDispatchSupport` e unboxing stubs por padrão, mais fallbacks de hardware intrinsics "otherwise, the interpreter won't have an implementation it can call for any non-ReadyToRun code" (`src/coreclr/tools/aot/crossgen2/Program.cs`). Isto mostra uma quarta opção: **R2R + interpretador** (ver "IL interpreter" mencionado em `tiered-compilation.md`).

## Por quê (quando escolher cada um)

### Escolha **JIT puro** quando
- o aplicativo é **dinâmico** (plugins carregados em runtime, `Reflection.Emit`, expression trees compiladas, `DynamicMethod`, scripting);
- o processo é **longo** e o desempenho de pico importa mais que o startup (serviços de longa duração com tráfego estável) — o tiering + PGO dinâmico dão o melhor pico;
- você precisa de **um só artefato portátil** (mesmo `.dll` em várias arquiteturas);
- tempo de build e simplicidade de deploy importam.

### Escolha **R2R** quando
- você quer **melhor startup** mas **preserva todo o dinamismo** do runtime (reflexão completa, carga de assemblies, `Reflection.Emit`);
- o app é grande e o JIT no startup domina (ferramentas CLI, aplicações desktop, serviços com cold start sensível, contêineres), e você aceita imagem maior;
- você pode controlar a **version bubble** (self-contained → composite com framework, `--inputbubble`) para ganhar mais desempenho;
- você precisa que o pico de desempenho ainda seja alto: R2R + tiering rejit chega perto do JIT puro.

### Escolha **Native AOT** quando
- **startup mínimo, tamanho/memória pequenos e ausência de runtime instalado** são requisitos (funções serverless, ferramentas CLI distribuídas como binário, contêineres minúsculos, dispositivos);
- a plataforma **proíbe geração de código em runtime** (iOS, consoles, WebAssembly com restrições);
- o código é **totalmente conhecido em build** (sem plugins/Emit; reflexão controlada);
- você aceita **sem PGO dinâmico** e sem JIT para específico de CPU (ou escolhe `IlcInstructionSet`);
- segurança/superfície reduzida (sem carregar IL dinamicamente).

### Heurística rápida

```
Precisa carregar código desconhecido em runtime?        -> JIT (ou R2R)
Startup é crítico, mas precisa de reflexão/dinamismo?   -> R2R
Startup/memória/tamanho críticos e mundo fechado?        -> Native AOT
Serviço de longa duração, pico > startup?                -> JIT (+ tiered PGO), opcionalmente R2R
Plataforma sem JIT?                                      -> Native AOT (ou R2R + interpretador)
```

## Trade-offs (resumo comparativo)

| Critério | JIT | R2R | Native AOT |
|---|---|---|---|
| Startup | ✗ (lento) | ✓ | ✓✓ |
| Pico de desempenho | ✓✓ (PGO dinâmico, vê CPU) | ✓ (via tiering) | ✓ (otimização global, sem PGO dinâmico) |
| Tamanho do artefato | ✓✓ (pequeno IL) | ✗ (IL + nativo) | ✓ (só alcançável; inclui runtime) |
| Dinamismo (Emit, Load, reflexão total) | ✓✓ | ✓✓ | ✗ |
| Portabilidade do binário | ✓✓ | ✗ (por plataforma) | ✗ (por RID) |
| Complexidade do build | ✓✓ | ✓ (passo crossgen2) | ✗ (compilação + link longos) |
| Resiliência a atualização de framework | ✓✓ | ✓ (garantia de versão do formato) | ✗ (recompilar para atualizar) |
| Depuração | ✓✓ (ferramentas gerenciadas) | ✓ | ✓ nativa (CodeView/DWARF) |
| Previsibilidade de latência (sem pausas de JIT) | ✗ | ✓ | ✓✓ |

Notas: "✓/✗" são avaliações relativas derivadas dos documentos citados, não métricas medidas por este texto.

## Como aplicar na sua linguagem

1. **Projete a IR para suportar os três modos**: IL/bytecode estável, metadata completa e uma "interface de VM" estreita entre o compilador e o runtime (como `ICorJitInfo`), de modo que o mesmo backend sirva JIT, AOT parcial e AOT total (é o que RyuJIT faz).
2. **Comece com o modo mais simples** (interpretador/JIT simples) e adicione tiers. A ordem histórica do .NET: JIT → NGEN → R2R (resiliente) → tiering → Native AOT.
3. **Escolha a granularidade de versionamento** (arquivo 05): se a linguagem quer atualizações independentes de bibliotecas, adote *version bubbles* e fixups; se quiser máxima velocidade e deploy fechado, vá de whole-program.
4. **Defina o "contrato de mundo fechado"** logo: o que a linguagem proíbe/permite dinamicamente. Marque APIs com atributos (`RequiresDynamicCode`, `RequiresUnreferencedCode`) e use *feature switches* (`IsDynamicCodeSupported`) para o código da biblioteca se adaptar ao modo.
5. **Dados de perfil**: ofereça perfil estático (`.mibc`-like) para AOT/R2R e dinâmico para JIT; use a mesma representação quando possível.
6. **Cache de código como camada**: construir o AOT como "preencher o cache do JIT" (R2R) dá um caminho de migração gradual e um fallback seguro (Native AOT não tem fallback).
7. **Tiering como política**: tier0 → tier1 com contadores e atraso de startup; inclua OSR para loops longos.
8. **Testes de equivalência**: mantenha uma suíte que rode os mesmos programas em JIT, R2R e AOT e compare resultados (o repositório tem `src/tests` com modos nativeaot e crossgen2).
9. **Diagnóstico**: perfmaps, mapas de objeto, grafo de dependências (`--dgmllog`, `IlcGenerateMapFile`) são essenciais para comparar o que cada modo gerou.
10. **Documente as diferenças semânticas** (ordem de module initializers, comportamento de reflexão, disponibilidade de APIs) entre modos.

## Referências

- `docs/design/features/tiered-compilation.md`, `OnStackReplacement.md`, `DynamicPgo.md`, `DynamicPgo-InstrumentedTiers.md`, `code-versioning.md`, `dotnet-pgo.md`.
- `src/coreclr/inc/clrconfigvalues.h` — `TieredCompilation`, `TC_*`.
- `docs/design/coreclr/jit/ryujit-overview.md`, `GuardedDevirtualization.md`.
- `docs/design/coreclr/botr/readytorun-overview.md`, `readytorun-format.md`, `ilc-architecture.md`.
- `docs/workflow/building/coreclr/nativeaot.md`, `src/coreclr/nativeaot/docs/*`.
- `src/coreclr/tools/aot/crossgen2/Program.cs` (`GetTargetAllowsRuntimeCodeGeneration`), `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeFeature.NativeAot.cs`.
- Arquivos 05, 06 e 07 desta série.
- ECMA-335 (o IL e a metadata são a base comum aos três modos).

## Apêndice A — Cenários concretos

| Cenário | Recomendação | Justificativa (baseada nos docs) |
|---|---|---|
| Microsserviço de longa duração com tráfego alto | JIT (com tiering + PGO), opcionalmente R2R do app/framework para reduzir cold start | O pico vem de tier1/PGO dinâmico; R2R apenas reduz o início |
| Ferramenta CLI de execução curta (ms) | Native AOT, ou R2R se precisar de reflexão/carga dinâmica | Sem JIT, startup mínimo |
| Aplicação com plugins carregados em runtime | JIT ou R2R | `Assembly.Load`/`Reflection.Emit` não funcionam em Native AOT |
| App em iOS ou navegador (WebAssembly) | Native AOT (ou R2R sem JIT + interpretador) | Plataformas sem codegen em runtime (`GetTargetAllowsRuntimeCodeGeneration` falso) |
| Contêiner pequeno / serverless | Native AOT | Sem runtime instalado, tamanho e memória menores |
| App de desktop grande, startup importante, dinâmico | R2R (possivelmente composite) | Mantém dinamismo e corta JIT no start |
| Biblioteca consumida por apps de todos os modos | Código-fonte compatível com trim/AOT, anotações `RequiresDynamicCode`/`RequiresUnreferencedCode` | Permite uso em qualquer dos três |

## Apêndice B — Como cada modo afeta os outros arquivos desta série

| Tópico | JIT | R2R | Native AOT |
|---|---|---|---|
| IL (01) | Consumido pelo JIT | Consumido pelo crossgen2 em build; IL mantido | Consumido pelo ILC; IL descartado |
| Metadata (02) | Completa | Completa | Substituída por metadata reduzida |
| Formato de arquivo (03) | PE/CLI | PE/CLI + `ManagedNativeHeader` | Objeto/executável nativo |
| Assinaturas (04) | Lidas em runtime | Lidas + estendidas para fixups | Resolvidas em build |
| Genéricos (08) | Dicionários em runtime, instanciação sob demanda | Pré-instanciados dentro da bolha; resto via JIT | Todas as instanciações descobertas em build |
| Debug (09) | PDB + mapa do JIT | PDB + `DebugInfo` + perfmap | CodeView/DWARF nativos |
| Module init / hooks (10) | `.cctor` do `<Module>` sob demanda; `ManagedStartup` | idem | Lista ordenada em build; hooks via `[ModuleInitializer]` |
| Strong name (11) | Identidade para binder | idem; `--compositekeyfile` em composite | Informativo |

## Apêndice C — Perguntas frequentes

- **"R2R e Native AOT geram o mesmo código?"** Usam o mesmo backend (RyuJIT) e infraestrutura (`src/coreclr/tools/aot`), mas com premissas diferentes: R2R precisa de resiliência de versão (fixups, sem inlining fora da bolha), AOT assume mundo fechado.
- **"Posso misturar?"** Sim, JIT+R2R no mesmo processo. Mistura de AOT total com JIT não existe como configuração de produção; o `ilc-architecture.md` apenas afirma que o design não a exclui ("one could imagine mixed modes of execution").
- **"Por que R2R não é mais rápido que JIT em pico?"** Porque o código R2R não pode assumir o que o JIT vê em runtime (CPU exata, dependências carregadas, perfil); por isso o tiering recompila o que é quente.
- **"E o tamanho?"** R2R aumenta o arquivo (IL + nativo); AOT reduz via análise global; JIT é o menor arquivo, mas precisa de runtime e memória para compilar.

## Apêndice D — Matriz de decisão por requisito

| Requisito dominante | JIT | R2R | Native AOT |
|---|:-:|:-:|:-:|
| Menor tempo até a primeira resposta | | ✓ | ✓✓ |
| Maior throughput sustentado | ✓✓ | ✓ | ✓ |
| Carregar código em runtime | ✓✓ | ✓✓ | ✗ |
| Menor consumo de memória residente | | | ✓✓ |
| Menor tamanho de distribuição "sem runtime" | | | ✓ |
| Plataforma sem JIT permitido | ✗ | ✓ (com interpretador) | ✓✓ |
| Mesmo binário em várias arquiteturas | ✓✓ | ✗ | ✗ |
| Atualização do framework sem recompilar o app | ✓✓ | ✓ | ✗ |

(Avaliação qualitativa derivada dos documentos citados.)
