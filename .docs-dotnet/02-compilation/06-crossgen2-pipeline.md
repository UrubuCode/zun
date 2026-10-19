# Crossgen2 — pipeline

## Mecanismo no .NET

Crossgen2 é o compilador antecipado (AOT) que produz imagens **ReadyToRun** (arquivo 05). Está escrito em C# e vive em `src/coreclr/tools/aot/`:

- `crossgen2/` — o executável: `Program.cs` (orquestração) e `Crossgen2RootCommand.cs` (opções de linha de comando, via System.CommandLine).
- `ILCompiler.ReadyToRun/` — a lógica específica de R2R: `Compiler/` (módulo-grupo, root providers, compilation builder), `Compiler/DependencyAnalysis/ReadyToRun/` (nós do grafo que viram seções do arquivo), `CodeGen/`, `JitInterface/`, `ObjectWriter/`, `IL/`, `Interop/`, `TypeSystem/`, `IBC/`.
- `ILCompiler.TypeSystem/` — sistema de tipos gerenciado compartilhado com o ILC (Native AOT): carrega assemblies ECMA-335 (`Internal.TypeSystem.Ecma`) e expõe `TypeDesc`/`MethodDesc`/`FieldDesc` (ver `docs/design/coreclr/botr/managed-type-system.md`).
- `ILCompiler.DependencyAnalysisFramework/` — motor genérico do grafo de dependências (também usado pelo ILC).
- `jitinterface/` — ponte para o JIT nativo (RyuJIT) via a interface `ICorJitInfo`.
- `ILCompiler.Reflection.ReadyToRun/` — leitor de imagens R2R (R2RDump etc.).

A base comum com o ILC é dita no `docs/workflow/building/coreclr/nativeaot.md`: *"The AOT compiler (ILC/ILCompiler) built on a shared codebase with crossgen2 (src/coreclr/tools/aot)."*

### Etapas (seguindo `Program.cs`)

1. **Parse de argumentos**. Entrada: `input-file-path` (um ou mais assemblies IL a compilar); `--reference/-r` (assemblies referenciáveis, não compilados); `--unrooted-input-file-paths/-u` (na bolha mas sem raízes); `--out/-o`; alvo: `--targetos`, `--targetarch`, `--instruction-set`, `--max-vectort-bitwidth`; otimização: `--optimize/-O`, `--optimize-disabled/--Od`, `--optimize-space/--Os`, `--optimize-time/--Ot`; perfil: `--mibc/-m`, `--embed-pgo-data`, `--partial`; modo: `--composite`, `--inputbubble`, `--inputbubbleref`, `--compilebubblegenerics`, `--non-local-generics-module`, `--opt-cross-module`; saída: `--obj-format/-f` (PE vs Mach-O objeto), `--rtr-header-symbol-name`, `--compositekeyfile`, `--map`, `--mapcsv`, `--pdb`, `--pdb-path`, `--perfmap`, `--perfmap-path`, `--perfmap-format-version`; diagnóstico: `--dgmllog`, `--verbose`, `--parallelism`, `--make-repro-path`, `--print-repro-instructions`, `--singlemethod*` (compilar um único método para repro); opções de strip: `--strip-inlining-info`, `--strip-debug-info`, `--strip-il-bodies`; layout: `--method-layout`, `--file-layout`, `--hot-cold-splitting`; verificações: `--verify-type-and-field-layout`, `--verify-gc-mode-transitions`, `--type-validation`. (Todas confirmadas em `Crossgen2RootCommand.cs`.)

2. **Contexto de tipos**. Cria `ReadyToRunCompilerContext(targetDetails, genericsMode, versionBubbleIncludesCoreLib, targetAllowsRuntimeCodeGeneration, targetIsSingleThreaded, instructionSetSupport, ...)`. Note que o alvo declara se permite geração de código em runtime: `GetTargetAllowsRuntimeCodeGeneration` retorna falso para iOS/tvOS/MacCatalyst/Browser/Wasi e para wasm32. Nesses casos ativa-se `EnableCachedInterfaceDispatchSupport` e `GenerateUnboxingStubs` por padrão, e adiciona-se `ReadyToRunHardwareIntrinsicRootProvider` para garantir implementações fallback dos intrínsecos de hardware (comentário no código: "the interpreter won't have an implementation it can call for any non-ReadyToRun code").

3. **Carga dos módulos**. Inputs, não-raiz e referências viram `EcmaModule`s (`typeSystemContext.GetModuleFromPath`). Calcula-se o *conjunto da bolha de versão* (`versionBubbleModulesHash`): em modo normal só o módulo de entrada; em `--composite` ou `--inputbubble`, todos os inputs (+ `--inputbubbleref`). Detecta se `System.Private.CoreLib` está na bolha (muda quais otimizações são legais).

4. **Grupo de compilação** (`ReadyToRunCompilationModuleGroupConfig` → `CompilationModuleGroup`). Decide, para cada tipo/método, se "versiona com" o módulo (pode ser inlinado, pode ter genérico compilado). Campos: `CompilationModuleSet`, `VersionBubbleModuleSet`, `IsCompositeBuildMode`, `IsInputBubble`, `CompileGenericDependenciesFromVersionBubbleModuleSet`, `CrossModuleInlining`, `DirectPInvokeModules`.

5. **Profile-guided (opcional)**. `ProfileDataManager` lê arquivos `.mibc` (parse de `MIbcProfileParser`). Em `--partial`, só métodos presentes no perfil são compilados; senão, aplica-se perfil como dica de layout/prioridade.

6. **Raízes (roots) da compilação**. Quais métodos começam o grafo:
   - `ReadyToRunProfilingRootProvider` (métodos do perfil);
   - `ReadyToRunVisibilityRootProvider` (membros visíveis publicamente) quando aplicável, ou `ReadyToRunLibraryRootProvider` (todos os métodos da biblioteca);
   - `ReadyToRunXmlRootProvider` (descritor XML embutido);
   - `ReadyToRunHardwareIntrinsicRootProvider` / `ReadyToRunJitHelperRootProvider`.

7. **Builder** (`ReadyToRunCodegenCompilationBuilder`): configura `ILProvider` (`ReadyToRunILProvider`), modo de otimização, tracking de dependências (`DependencyTrackingLevel` — `None`, `First` ou `All` se `--dgmllog`), `NodeFactoryOptimizationFlags`, escritor de PDB/perfmap/map, layout de arquivo (`FileLayoutOptimizer`, `PettisHansenSort` em `Compiler/PettisHansenSort`), caminho do JIT (`--jitpath`), `UseResilience` (`--resilient`: continuar quando um método falha), `UseGenericCycleDetection(depthCutoff, breadthCutoff)` (corta instanciações genéricas infinitas).

8. **Compilação** (`compilation.Compile(outFile)`):
   - O **grafo de dependências** (`ILCompiler.DependencyAnalysisFramework`) é expandido a partir das raízes: cada método compilado gera dependências (tipos, outros métodos, fixups). Nós em `Compiler/DependencyAnalysis/ReadyToRun/` representam as seções finais (import sections, runtime functions, method entry points...).
   - Para cada método, o **JIT nativo (RyuJIT)** é carregado como biblioteca (`clrjit`), e o crossgen2 implementa o lado "VM" via `jitinterface` (callbacks `ICorJitInfo`: resolver tokens, layout de tipos, perguntar se inlining é legal no *version bubble*, registrar fixups). O JIT devolve código nativo + GC info + unwind + debug info. Em R2R, as respostas do compilador à JIT-EE são *restritas* ao que é estável sob versionamento (por isso o código resultante usa os slots/fixups do arquivo 05).
   - Em seguida o **object writer** (`ObjectWriter/`) serializa seções: código, dados, `RuntimeFunctions`, `MethodDefEntryPoints`, `ImportSections`, etc., produzindo um PE (ou objeto Mach-O com `--obj-format macho`, ver `readytorun-platform-native-envelope.md`).
   - `compilation.DeterminismCheckFailed` verifica a reprodutibilidade (há `--determinism-stress`).

9. **Pós-processo**: gera `--map`, `--pdb`/`--perfmap` (arquivos `.r2rmap` + entrada de Debug Directory tipo 21), e `--dgmllog` com o grafo de dependências.

### Modos relevantes

| Modo | Efeito | Fonte |
|---|---|---|
| Single-assembly (padrão) | Cada `.dll` vira um `.dll` R2R; bolha = o assembly | `Program.cs` |
| `--inputbubble` | Todos os inputs na mesma bolha; inlining/genéricos cruzam módulos | `Crossgen2RootCommand.cs` |
| `--composite` | Um binário R2R com todos os inputs; `--compositerootpath`, `--compositekeyfile` | idem |
| `--partial` | Só o que o perfil `.mibc` pede | idem |
| `--compile-no-methods` | Gera imagem sem corpos (apenas estruturas) | idem |
| `--singlemethod*` | Compila um método (diagnóstico/repro) | idem |
| `--opt-cross-module` | Permite inline de código de módulos fora da bolha, com validação | idem |

### Heurísticas de geração (documentadas)

`docs/design/features/crossgen2-compilation-structure-enhancements.md`: princípios — código AOT existe para melhorar startup; por padrão confia-se em *rejit* via tiering; "Too much pregenerated code will negatively affect applications" (equilíbrio entre tempo de ler o arquivo e tempo de compilar). Código não genérico: todo ou apenas o do perfil; genéricos: gating por orçamento relativo ao código não genérico; heurísticas propostas (async state machines, `MoveNext`/`Current` de iteradores, métodos de interface usados por LINQ, instanciações estaticamente descobertas em `typespec`/`methodspec` quando a bolha ≥ Application). Para evitar duplicação de genéricos entre camadas, propõe-se **composite por camada**.

Regras de bolha citadas ali: inlining só dentro da bolha (exceto `NonVersionable`); instanciação genérica só entra se a definição *e* os argumentos estão na bolha — com exceção de `System.__Canon`. Exemplo do doc: `MyGeneric<int>` fica na bolha de `MyGeneric`; `MyGeneric<DateTime>` só se `MyGeneric` compartilha bolha com `System.Private.CoreLib`.

### Integração SDK

O SDK publica R2R chamando `crossgen2` ("As of .NET 6, the .NET SDK can publish applications as ReadyToRun using the crossgen2 tool" — `readytorun-overview.md`); no runtime, suporte sob `FEATURE_READYTORUN`; "RyuJIT is used as the code generator". Projetos: `crossgen2.csproj`, `crossgen2_inbuild.csproj`, `crossgen2_publish.csproj`, `ILCompiler.ReadyToRun.csproj`.

## Por quê

- **Gerenciado e portável**: escrever o compilador em C# sobre `System.Reflection.Metadata` permite *cross-compile* (compilar para arm64 em x64 etc.) com um único conjunto de ferramentas.
- **Reuso do JIT**: a qualidade de codegen é a do RyuJIT sem manter um segundo backend; a diferença é só o "lado VM" (`jitinterface`), que o crossgen2 reimplementa em C# restringindo respostas a fatos estáveis por versão.
- **Grafo de dependências**: só compila o que é alcançável a partir das raízes e só emite as estruturas necessárias; também alimenta `--dgmllog` para diagnóstico ("WhyDgml" em `tools/aot/WhyDgml`).
- **Determinismo e paralelismo**: grafo + tarefas por método permitem `--parallelism` e builds reproduzíveis.

## Trade-offs

| Decisão | Ganho | Custo |
|---|---|---|
| Reimplementar o "VM side" em C# | Cross-compilação; sem dependência do runtime alvo | Duplicação de lógica do runtime (type layout, field layout — cf. `ReadyToRunMetadataFieldLayoutAlgorithm`, `--verify-type-and-field-layout` serve de rede de segurança) |
| Compilar o que a bolha permite | Segurança de versão | Menos inlining/menos genéricos pré-compilados |
| Perfil `.mibc` | Escolhe o que pré-compilar, melhora layout | Exige coletar perfis (`dotnet-pgo`, `docs/design/features/dotnet-pgo.md`) |
| Composite | Máximo desempenho | Acoplamento e ordem de deploy |
| Pré-gerar P/Invoke | Startup | Subconjunto de marshallers duplicado em C# |
| Resilient mode | Continua após falha em um método | Pode mascarar erros se mal usado |

## Como aplicar na sua linguagem

1. **Divida o compilador AOT em**: (a) carregador de módulos + sistema de tipos, (b) grupo de compilação (política: o que pode ser otimizado junto), (c) provedores de raízes, (d) grafo de dependências, (e) backend de código, (f) escritor de formato final. É exatamente a decomposição do `ILCompiler.*`.
2. **Torne o "version bubble" um objeto de primeira classe** (`CompilationModuleGroup`) que responde: "este método pode ser inlinado aqui?", "esta instanciação genérica pertence a este módulo?".
3. **Use análise de dependências** em vez de compilar tudo: nós "de objeto" (viram bytes) e nós "abstratos" (guardam fatos, como "existe chamada virtual para X"), com dependências condicionais/dinâmicas.
4. **Interface backend/VM estreita** (modelo `ICorJitInfo`): o backend pergunta "layout do tipo", "qual o alvo da chamada", "devo registrar um fixup"; o frontend responde. Com isso você troca backend (JIT, LLVM, C) sem tocar o resto.
5. **Cross-compile e reprodutibilidade** desde o dia 1 (flag de determinismo e teste de estresse).
6. **Comandos de diagnóstico**: `--map`, `--dgmllog`, `--singlemethod*`, `--make-repro-path`. Eles economizam semanas de depuração.
7. **Feature flags por alvo** (permite codegen em runtime? single thread?): determine fallbacks no momento da compilação.
8. **Perfil como entrada opcional** que influencia raízes e layout, não a corretude.

## Referências

- `src/coreclr/tools/aot/crossgen2/Program.cs`, `Crossgen2RootCommand.cs`, `crossgen2.csproj`.
- `src/coreclr/tools/aot/ILCompiler.ReadyToRun/Compiler/` — `ReadyToRunCodegenCompilation(.Builder).cs`, `ReadyToRunCompilationModuleGroupBase.cs`, `ReadyToRun*RootProvider.cs`, `ReadyToRunCompilerContext.cs`, `FileLayoutOptimizer.cs`, `ProfileDataManager.cs`.
- `src/coreclr/tools/aot/ILCompiler.DependencyAnalysisFramework/`, `jitinterface/`, `ILCompiler.TypeSystem/`, `ILCompiler.Reflection.ReadyToRun/`.
- `docs/design/features/crossgen2-compilation-structure-enhancements.md`, `readytorun-composite-format-design.md`, `dotnet-pgo.md`.
- `docs/design/coreclr/botr/readytorun-overview.md`, `readytorun-format.md`, `managed-type-system.md`, `ilc-architecture.md` (arquitetura comum de dependências).
- ECMA-335 (formato de entrada: IL + metadata).

## Apêndice A — Exemplos de uso (compostos a partir das opções reais)

Compilar um assembly em modo padrão (um `.dll` → um `.dll` R2R), referenciando o framework:

```
crossgen2 MyApp.dll -o out/MyApp.dll -r <dir-do-framework>/*.dll --targetos linux --targetarch x64 -O
```

Compilar vários assemblies como **imagem composta** (um binário nativo único):

```
crossgen2 --composite --inputbubble A.dll B.dll -o out/ -r <framework>/*.dll --targetos linux --targetarch x64
```

Compilar só o que o perfil pede (`--partial`) com perfil `.mibc`:

```
crossgen2 MyApp.dll --partial --mibc app.mibc -o out/MyApp.dll -r <framework>/*.dll
```

Reproduzir problema em um único método:

```
crossgen2 MyApp.dll --singlemethodtypename "My.Type" --singlemethodname "Run" --singlemethodindex 0 -o out/x.dll -r ...
```

Diagnóstico: `--map`, `--mapcsv`, `--dgmllog graph.dgml`, `--verbose`, `--make-repro-path`. (As formas acima são ilustrativas; os nomes das opções vêm de `Crossgen2RootCommand.cs`. Ajuste caminhos e confira `crossgen2 --help` para a versão em uso.)

## Apêndice B — Fluxo de dados resumido

```
 .dll IL (+ referências)       .mibc (opcional)
        |                           |
        v                           v
  EcmaModule (TypeSystem) ---> ProfileDataManager
        |                           |
        v                           v
  CompilationModuleGroup  <---- política de bolha
        |
   roots (Profiling/Visibility/Library/Xml/HardwareIntrinsic)
        |
        v
  Dependency graph  --- para cada método --> JIT (RyuJIT) via jitinterface
        |                                         |
        |                       código + GC info + unwind + debug info + fixups
        v
  ObjectWriter (PE / Mach-O) ---> .dll R2R / composite + .r2rmap/.pdb opcionais
```

## Apêndice C — Pontos delicados descobertos no código

1. **Alvos sem codegen em runtime** (`GetTargetAllowsRuntimeCodeGeneration`): o compilador troca políticas padrão (stubs de unboxing e dispatch em cache) — o modelo de "R2R como cache" deixa de ser opcional e passa a ser "a única fonte de código".
2. **Tamanho do Vector\<T\>** (`--max-vectort-bitwidth`) e **conjunto de instruções** (`--instruction-set`) entram na compatibilidade do código gerado; o tema é tratado em `docs/design/features/hw-intrinsics.md` (e em `docs/design/coreclr/botr/vectors-and-intrinsics.md`).
3. **Determinismo**: `--determinism-stress` repete a compilação para provar reprodutibilidade; falha → `"Determinism Check Failed"`.
4. **Ciclos genéricos**: `--maxgenericcycle` e `--maxgenericcyclebreadth` evitam expansão infinita de instanciações (`UseGenericCycleDetection`).
5. **Layout de arquivo**: `--method-layout` e `--file-layout` (com `PettisHansenSort`) organizam métodos quentes/frios para melhorar localidade de página em startup.
6. **Strip**: `--strip-inlining-info`, `--strip-debug-info`, `--strip-il-bodies` reduzem a imagem ao custo de capacidades (rejit/profiler/depuração).
