# Native AOT — modelo e limitações

## Mecanismo no .NET

Native AOT compila um programa .NET **inteiro, antes da execução**, para um executável nativo da plataforma (COFF/PE, ELF ou Mach-O) que **não carrega IL nem usa JIT**. É uma versão enxuta do CoreCLR (runtime reduzido) ligada estaticamente ao código gerado. `docs/workflow/building/coreclr/nativeaot.md` ("High Level Overview") lista os componentes:

- **ILC / ILCompiler** (`src/coreclr/tools/aot/ILCompiler*`): *"built on a shared codebase with crossgen2"*. Onde crossgen2 gera módulos R2R para o CoreCLR, o ILC *"generates code and self-describing datastructures for a stripped down version of CoreCLR into object files"* (COFF+CodeView no Windows, ELF+DWARF no Linux, Mach-O+DWARF no macOS).
- **Runtime reduzido** (`src/coreclr/nativeaot/Runtime`, mais arquivos compartilhados de `src/coreclr`): GC, tratamento de exceções, stack walking, `MethodTable`, `TypeManager`, `RuntimeInstance` etc., compilado como biblioteca estática.
- **Bootstrapper** (`src/coreclr/nativeaot/Bootstrap`): pequeno código nativo com o `main()` real; dois sabores (executável e biblioteca dinâmica).
- **Bibliotecas centrais** (`src/coreclr/nativeaot`): `System.Private.CoreLib`, `System.Private.Reflection.*` (reflexão baseada em metadata gerada), `System.Private.TypeLoader` (carregar tipos não gerados estaticamente), `System.Private.StackTraceMetadata`.
- **Integração com `dotnet publish`** (`src/coreclr/nativeaot/BuildIntegration`: `.targets/.props`), que roda o ILC e o linker da plataforma (`link.exe` no Windows, `ld` em Linux/macOS).

O resultado: um executável autocontido ("standalone, modulo any managed DllImports"), depurável com depuradores nativos com "full-fidelity access to locals, and stepping information".

### Arquitetura do compilador (ILC)

De `docs/design/coreclr/botr/ilc-architecture.md`:

- **Driver → Compiler → backends**. O driver configura um `CompilationBuilder` e obtém uma `Compilation`. Backend atual: **RyuJIT** (x64, arm64, arm32 em Windows/Linux/macOS/BSD); o backend **LLVM** (WebAssembly) vive em outro branch (runtimelab); o antigo `CppCodegen` (C++) não foi trazido do CoreRT.
- **Análise de dependências** (conceito central): grafo de nós de objeto (bytes na saída) e nós abstratos (fatos como "há chamada virtual a Object.GetHashCode"). Arestas = "requer". Parte das **raízes** (`ICompilationRootProvider`, ex.: `Main`) e é limitada por um **grupo de compilação** (`CompilationModuleGroup`). Três tipos de dependência: **estáticas**, **condicionais** (B só se C está no grafo) e **dinâmicas** (caras; usadas para *generic virtual methods*).
- Exemplo do doc: vtable com entradas condicionais — `Bar::UnusedVirtualMethod` nunca é compilado porque ninguém chama `Foo::UnusedVirtualMethod`; `Baz` tem um nó "unconstructed type" (só para cast), sem vtable; `Baz::VirtualMethod` não é compilado porque `Baz` nunca é alocado.
- **IL scanning**: uma "pré-compilação" com backend nulo que constrói o grafo sem gerar código, para que a compilação real conheça o programa todo (ex.: atribuir slots de vtable lazy e então embuti-los nos call sites). Opcional (`IlcScanReflection` etc. nas props).
- **Object writing**: gera arquivos de objeto na forma da plataforma.
- **Acoplamento com a BCL**: formato do `MethodTable`, blobs de metadata, e métodos bem-conhecidos (helpers de overflow, de marshalling). Acoplamento "relativamente frouxo": `Test.CoreLib` (`src/coreclr/nativeaot/Test.CoreLib`) é uma BCL mínima para trazer NativeAOT a novas plataformas.
- **Genéricos**: compartilhamento canônico (`__Canon`) só para tipos-referência; valor ⇒ corpos separados; `RuntimeDeterminedType`, *shadow method nodes*, GVMs via nós dinâmicos (ver arquivo 08).
- **Corpos gerados pelo compilador**: helpers IL sintetizados (delegates, marshalling de P/Invoke, `ValueType.Equals/GetHashCode`, reflexão). `--ildump` / `IlcDumpGeneratedIL` os despeja.

### Mundo fechado

*"An advantage of a fully ahead of time compiled environment is that the compiler can make closed world assumptions... lacking the ability to load arbitrary CIL at runtime (either through `Assembly.Load`, or `Reflection.Emit`), if the compiler sees that there's only one type implementing an interface, it can replace all the interface calls ... with direct calls"* (`ilc-architecture.md`, "Optimization pluggability"). Essa é a raiz tanto das otimizações quanto das limitações.

### Startup

`StartupCodeMainMethod` (`src/coreclr/tools/aot/ILCompiler.Compiler/IL/Stubs/StartupCode/`) é um método **sintetizado em IL** que, em ordem: chama initializers de biblioteca; `InitializeCommandLineArgs[W]`; `InitializeEntryAssembly`; `InitializeApartmentState` (STA se `[STAThread]`, senão MTA); `RunModuleInitializers`; então o `Main` do programa. A lista de module initializers é montada em tempo de compilação por `ModuleInitializerListNode` (ver arquivo 10). Para bibliotecas nativas existe `NativeLibraryStartupMethod`.

### Metadata e reflexão

Em vez do metadata ECMA-335, o ILC gera um **formato próprio mais compacto** (projeto `ILCompiler.MetadataTransform`) apenas para o que as políticas de reflexão pedem, mais *stack trace metadata* (`System.Private.StackTraceMetadata`). O compilador "may miss types if an application uses reflection", daí o suporte a `rd.xml` (`src/coreclr/nativeaot/docs/rd-xml-format.md` — formato interno e *unsupported* fora do repo) e a análise de padrões de reflexão. Propriedades do build (`Microsoft.NETCore.Native.targets`): `IlcGenerateCompleteTypeMetadata`, `IlcTrimMetadata`, `IlcScanReflection`, `IlcRootedAssemblies`, `IlcTrimmedAssemblies`, `IlcDehydrate`, `IlcFoldIdenticalMethodBodies`, `IlcInstructionSet`, `IlcMaxVectorTBitWidth`, `IlcMibcPath`, `IlcPgoOptimize`, `IlcMultiModule`, `IlcExportUnmanagedEntrypoints`, `IlcSingleThreaded`, `IlcUseServerGc`, `IlcDumpIL`, `IlcGenerateMapFile`, `IlcGenerateDgmlFile`, `IlcGenerateMstatFile`.

Diagnóstico (`src/coreclr/nativeaot/docs/troubleshooting.md`): log de metadata CSV, DGML do grafo (`*.codegen.dgml.xml`, `*.scan.dgml.xml`), mapa de objetos (`*.map.xml`), IL gerado, `mstat` (tamanho por tipo/método).

### Opções de ajuste

`optimizing.md`: `<IlcInstructionSet>` (por padrão mira o conjunto mínimo da plataforma; `native` usa a CPU atual, sem suporte a cross-compile), `<IlcMaxVectorTBitWidth>` (tamanho de `Vector<T>`).

### Limitações (confirmadas no código)

| Limitação | Evidência |
|---|---|
| Sem geração de código dinâmica | `RuntimeFeature.IsDynamicCodeSupported => false` e `IsDynamicCodeCompiled => false` (`nativeaot/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeFeature.NativeAot.cs`) |
| `Reflection.Emit` não suportado | `AssemblyBuilder.DefineDynamicAssembly` chama `ReflectionEmitThrower.ThrowPlatformNotSupportedException()` (`.../System/Reflection/Emit/AssemblyBuilder.cs`); também `DynamicMethod`, `SignatureHelper`, `CustomAttributeBuilder`, `DynamicILInfo` |
| Sem carga de assemblies arbitrários | `AssemblyLoadContext.InternalLoadFromPath` e `InternalLoad(rawAssembly...)` lançam `PlatformNotSupportedException` (`AssemblyLoadContext.NativeAot.cs`); `GetLoadContext` retorna sempre o `Default` |
| Hot reload / `MetadataUpdater` | há um `System/Reflection/Metadata/MetadataUpdater.cs` próprio no CoreLib do NativeAOT (não inspecionado em detalhe; não assuma suporte) |
| Reflexão só do que foi gerado | `rd-xml-format.md` e análise do compilador |
| Plataformas: depende de backend e de PAL | Windows/Linux/macOS (e BSD) via RyuJIT; WebAssembly via LLVM em outro branch |
| Compilação por módulo separado (`IlcMultiModule`) não é configuração de produção | `nativeaot.md`: *"It's not a shipping configuration and has many problems (requires exactly matching compilation settings, forfeits many optimizations, and has trouble around cross-module generic virtual method implementations)"* |
| Instrução SIMD mínima por padrão; `native` impede cross-compile | `optimizing.md` |

Também: `System.StartupHookProvider` usa `[ModuleInitializer]` e lê `DOTNET_STARTUP_HOOKS` apenas se `IsSupported` (feature switch) — ver arquivo 10.

### Trimming implícito

Como só o alcançável entra no grafo, Native AOT é também um *tree shaker* global. `ILCompiler.Trimming.Tests` testa isso. Avisos de trim/AOT são emitidos para APIs com `[RequiresUnreferencedCode]`/`[RequiresDynamicCode]` (visíveis no código: `AssemblyBuilder.DefineDynamicAssembly` tem `[RequiresDynamicCode("Defining a dynamic assembly requires dynamic code.")]`).

## Por quê

- **Startup mínimo e sem JIT**: nada para compilar ou carregar. Melhora tempo de início e *working set*.
- **Sem runtime instalado**: executável independente (útil em contêineres, serverless, ferramentas CLI, e em plataformas que proíbem JIT).
- **Tamanho pequeno**: o grafo de dependências elimina código e metadata não usados; `IlcFoldIdenticalMethodBodies` e `IlcDehydrate` reduzem mais.
- **Otimização de programa inteiro**: devirtualização, vtables sob demanda, inlining com conhecimento global.
- **Plataformas sem codegen em runtime** (iOS, WebAssembly): o único jeito de rodar .NET.
- **Superfície de ataque menor**: sem carregamento dinâmico de código.

## Trade-offs

| Ganho | Custo |
|---|---|
| Sem JIT, startup rápido | Sem tiering/PGO dinâmico: o código final é fixo na compilação (opção: PGO estático `.mibc`) |
| Mundo fechado ⇒ otimizações fortes | Sem `Assembly.LoadFrom`, `Reflection.Emit`, `DynamicMethod`, plugins |
| Tamanho menor | Reflexão e serialização por reflexão exigem análise/anotações/rd.xml; erros aparecem em runtime se o compilador não previu |
| Executável autocontido | Um binário por RID/arquitetura; tempo de compilação alto (build + link) |
| Debug nativo | O doc descreve depuração com depuradores nativos (símbolos CodeView/DWARF), não pelo pipeline de debug gerenciado |
| Runtime enxuto | Diferenças de comportamento/compatibilidade (menos APIs, `IsDynamicCodeSupported=false`) |
| Modelo de GC e exceções iguais | Cada nova plataforma exige portar `Runtime/` (Pal, EH, thunks) |

## Como aplicar na sua linguagem

1. **Decida cedo se "mundo fechado" é um objetivo**. Se for, projete a linguagem para que o compilador consiga provar o que é alcançável: evite (ou torne explícito/anotado) `eval`, reflexão total, `load` dinâmico. Trate APIs "não-AOT-safe" com atributos (`RequiresDynamicCode`, `RequiresUnreferencedCode`) e emita warnings no compilador.
2. **Implemente o compilador como análise de dependências** com raízes + grupo de compilação + dependências condicionais. Esse mecanismo dá, quase de graça: tree-shaking, vtables sob demanda e métodos virtuais só quando há uso.
3. **Faça um passo de "scan" sem codegen** antes do passo real, para obter conhecimento global (layout de vtables, devirtualização).
4. **Separe "metadata de execução" da "metadata de compilação"**: gere uma versão reduzida para reflexão, com política configurável (nada, só nomes, completa) e um arquivo de diretivas para forçar inclusões.
5. **Mantenha um runtime mínimo** (GC, EH, tipos, threads) cujo contrato com a biblioteca padrão seja pequeno e testável — como `Test.CoreLib`. Isso facilita novas plataformas.
6. **Gere startup como código sintetizado** (ordem: inicializadores de módulo → args → `main`), em vez de embutir lógica no bootstrapper nativo.
7. **Produza objetos nativos padrão** (COFF/ELF/Mach-O) e use o linker do sistema; ganha-se depuradores, perfiladores e assinatura de código.
8. **Ofereça diagnóstico de tamanho e de grafo** (DGML, mapa, estatísticas por método). Sem isso o usuário não consegue entender por que o binário cresceu.
9. **Para genéricos**, defina cedo a estratégia (compartilhar para refs, especializar para valores) e como ela interage com reflexão e dicionários (arquivo 08).
10. **Combine com um modo JIT/interpretado** se quiser dinamismo: o ILC admite "mixed modes", segundo o doc ("ahead of time compilation does not preclude just in time compilation").

## Referências

- `docs/workflow/building/coreclr/nativeaot.md` — visão geral dos componentes e build.
- `docs/design/coreclr/botr/ilc-architecture.md` — arquitetura do ILC, grafo, genéricos, GVM, IL scanning.
- `src/coreclr/nativeaot/docs/` — `README.md`, `compiling.md`, `optimizing.md`, `troubleshooting.md`, `rd-xml-format.md`.
- `src/coreclr/nativeaot/` — `Runtime/`, `Runtime.Base/`, `Bootstrap/`, `System.Private.CoreLib/`, `System.Private.Reflection.Execution/`, `System.Private.TypeLoader/`, `System.Private.StackTraceMetadata/`, `Test.CoreLib/`, `BuildIntegration/`.
- `src/coreclr/tools/aot/` — `ILCompiler/`, `ILCompiler.Compiler/`, `ILCompiler.RyuJit/`, `ILCompiler.MetadataTransform/`, `ILCompiler.Trimming.Tests/`, `ILCompiler.DependencyAnalysisFramework/`, `DependencyGraphViewer/`.
- `src/coreclr/tools/aot/ILCompiler.Compiler/IL/Stubs/StartupCode/StartupCodeMainMethod.cs`; `.../DependencyAnalysis/ModuleInitializerListNode.cs`.
- Documentação oficial referenciada: https://learn.microsoft.com/dotnet/core/deploying/native-aot/.

## Apêndice A — Fluxo de build `dotnet publish` com Native AOT

1. O SDK compila C# para IL normalmente (Roslyn).
2. Os targets em `src/coreclr/nativeaot/BuildIntegration` montam um **response file** do ILC (`*.ilc.rsp`, propriedade `IlcRspFile`), contendo assemblies de entrada, referências (`IlcReference`), caminhos de SDK/framework (`IlcSdkPath`, `IlcFrameworkPath`, `IlcFrameworkNativePath`), opções de instrução (`IlcInstructionSet`), perfis (`IlcMibcPath`), etc.
3. O **ILC** roda: IL scanning (opcional) → compilação com dependência → escrita do objeto (`IlcOutputFile`).
4. O **linker nativo** da plataforma liga o objeto gerado com as bibliotecas estáticas do runtime reduzido e as bibliotecas nativas do framework (por exemplo, a PAL e interop) para formar o executável final.
5. Para depurar o compilador com o mesmo input: abrir `ilc.slnx`, usar o `.rsp` como argumento (`@obj\Release\net8.0\win-x64\native\HelloWorld.ilc.rsp`, conforme `nativeaot.md`).

## Apêndice B — O que o runtime reduzido ainda precisa fazer em execução

Mesmo sem JIT, existe trabalho em tempo de execução (`src/coreclr/nativeaot/Runtime`):

- **GC** (arquivos `GCHelpers.cpp`, `GcEnum.cpp`, `GCMemoryHelpers.cpp`; GC compartilhado com o CoreCLR);
- **Tratamento de exceções e stack walking** (`EHHelpers.cpp`, `StackFrameIterator.cpp`), usando as tabelas de EH e GC info geradas pelo ILC;
- **Tipos** (`MethodTable.cpp`, `TypeManager.cpp`, `RuntimeInstance.cpp`): estruturas geradas estaticamente;
- **Thunks** (`ThunksMapping.cpp`) e helpers por arquitetura (`amd64/`, `arm/`, `arm64/`);
- **Finalização, handles, sync clean** (`FinalizerHelpers.cpp`, `HandleTableHelpers.cpp`, `SyncClean.cpp`);
- **Eventos** (`EnabledEventPipeInterface.cpp` / `DisabledEventPipeInterface.cpp`) e configuração (`RhConfig.cpp`).

O **carregador de tipos em runtime** (`System.Private.TypeLoader`) existe para casos em que um tipo (por exemplo, uma instanciação genérica não prevista estaticamente) precisa ser construído em execução usando templates gerados em build — é a parte "dinâmica" limitada que o modelo permite.

## Apêndice C — Checklist de portabilidade para uma nova plataforma (derivado dos docs)

1. Backend de código para a arquitetura (RyuJIT já suporta x64, arm64, arm32; LLVM para Wasm em outro branch).
2. Formato de objeto e debug da plataforma (COFF+CodeView, ELF+DWARF, Mach-O+DWARF).
3. Portar `Runtime/` (PAL, thunks, helpers de montagem por arquitetura) — o `Test.CoreLib` serve como BCL mínima de bring-up ("requires almost no initialization").
4. Integração de build (`BuildIntegration`): detecção de linker, flags, bibliotecas nativas.
5. Decidir o comportamento de recursos dinâmicos (como `IsDynamicCodeSupported`, que no NativeAOT é `false`).
6. Validar com os testes de fumaça do NativeAOT (`src/tests/build.sh -nativeaot ... -tree:nativeaot`, conforme `nativeaot.md`).

## Apêndice D — Políticas configuráveis do compilador (resumo do `ilc-architecture.md`)

O ILC mantém o núcleo "livre de política"; o driver injeta políticas que determinam o resultado:

| Política | Pergunta que responde | Exemplo concreto no doc |
|---|---|---|
| Raízes (`ICompilationRootProvider`) | Por onde começa o grafo? | `Main`, exports de biblioteca, modo método único |
| Grupo de compilação (`CompilationModuleGroup`) | Até onde o grafo se expande? | Arquivo único × multi-arquivo |
| Geração de vtables (`VTableSliceProvider`) | Vtables completas ou só com os slots usados? | Lazy exige suposições de mundo fechado |
| Metadata de reflexão | Que tipos/métodos/campos terão metadata? | `IlcTrimMetadata`, `IlcGenerateCompleteTypeMetadata` |
| Devirtualização | Quando trocar chamada virtual por direta? | Se há uma única implementação conhecida |
| Stack trace metadata | Nomes legíveis em `Exception.ToString`? | `System.Private.StackTraceMetadata` |
| Informação de debug | Que símbolos nativos gerar? | CodeView/DWARF |
| Fonte de IL | De onde vêm os corpos de método? | `ILProvider`; corpos gerados por `ILEmitter` |

Isto serve de modelo de arquitetura: um **motor neutro** + **políticas plugáveis** permite reutilizar o mesmo código em R2R (crossgen2 usa outro conjunto de políticas) e Native AOT.

## Apêndice E — Perguntas que o autor de uma linguagem deve responder antes de ter AOT

1. A linguagem tem *eval*, carga de código ou geração dinâmica? Se sim, o AOT as proíbe, as emula (interpretador) ou as restringe por anotação?
2. A reflexão enumera membros por nome (`GetMethod("X")`)? Como o compilador descobre isso estaticamente? (.NET: análise de padrões + `rd.xml`/atributos.)
3. Há genéricos reificados? Quantas instanciações podem surgir em runtime (ex.: via reflexão `MakeGenericType`)? (.NET: `System.Private.TypeLoader` com templates.)
4. Como são resolvidos os inicializadores estáticos e de módulo (arquivo 10)? Eager ou lazy? Em que ordem?
5. Que parte da biblioteca padrão depende do SO, e qual o contrato mínimo para uma nova plataforma (`Test.CoreLib`)?
6. Como o usuário diagnostica "por que meu binário ficou grande" ou "por que esse tipo foi removido"? (.NET: DGML, mstat, mapa.)
