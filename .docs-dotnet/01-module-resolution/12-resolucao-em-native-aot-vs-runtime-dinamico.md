# Resolucao em Native AOT vs runtime dinamico

> Objetivo: contrastar o modelo "resolver em tempo de execucao" (CoreCLR + host + binder + ALC) com o modelo "resolver em tempo de compilacao" (NativeAOT: ILCompiler fecha o mundo, o runtime so consulta metadados embutidos), e extrair o que isso implica para uma linguagem que queira suportar os dois modos.

## Mecanismo no .NET

### Duas respostas diferentes para "onde esta o modulo X?"

| Pergunta | CoreCLR (JIT/R2R) | NativeAOT |
|---|---|---|
| Quando as referencias sao resolvidas? | Primeiro uso, em execucao (arquivos 03-05, 10) | **Em tempo de compilacao**, pelo `ILCompiler` |
| Quem encontra os arquivos? | `hostfxr`/`hostpolicy` + binder (`src/native/corehost`, `src/coreclr/binder`) | Argumentos do compilador: `--reference/-r`, `--satellite` (`src/coreclr/tools/aot/ILCompiler/ILCompilerRootCommand.cs`) |
| Existe `deps.json`/`runtimeconfig.json` em runtime? | Sim (host le) | Nao ha host gerenciando frameworks nem roll-forward (o runtime vem embutido no binario) |
| Contextos de carga | Default + N customizados | Somente o Default (stub) |
| Carregar codigo novo em execucao | `LoadFromAssemblyPath`, `Reflection.Emit`, ... | Nao (ver abaixo) |
| Unidade de distribuicao | Muitos arquivos (`.dll`) + manifesto | Um executavel nativo (ou lib nativa) |

### O que e "resolucao" em NativeAOT

O `ILCompiler` (`src/coreclr/tools/aot/ILCompiler/`) recebe os assemblies de entrada e as referencias (`ReferenceFiles`, mapa simplename->caminho), resolve todo o grafo, executa trimming/analise de alcancabilidade e emite um unico binario. As referencias a assemblies sao **eliminadas**: tipos e metodos viram simbolos do binario; metadados residuais (para reflexao) sao um subconjunto embutido em formato proprio ("NativeFormat"). Opcoes relevantes em `ILCompilerRootCommand.cs`:

- `--reference/-r`: arquivos de referencia (a "TPA" do compilador);
- `--satellite`: assemblies satelites associados a entradas/referencias;
- `--multifile`: compila so os arquivos de entrada, sem os referenciados (o doc da opcao: "Compile only input files (do not compile referenced assemblies)");
- `--directpinvoke`, `--directpinvokelist`: P/Invokes chamados diretamente (linkados estaticamente, sem busca em runtime);
- `--nativelib`: gera biblioteca estatica/compartilhada.

### O "binder" em runtime e uma consulta a metadados embutidos

Se alguem ainda chama `Assembly.Load(name)` em NativeAOT, o caminho e `ReflectionAugments.Load` (`src/coreclr/nativeaot/System.Private.CoreLib/src/Internal/Reflection/Augments/ReflectionAugments.cs`, linha ~97) -> `RuntimeAssemblyInfo.GetRuntimeAssembly[IfExists]` -> `Dispensers.cs` (`s_assemblyRefNameToAssemblyDispenser`) -> `ReflectionCoreExecution.ExecutionEnvironment.AssemblyBinder.Bind(refName, ...)`, implementado em `src/coreclr/nativeaot/System.Private.TypeLoader/src/Internal/Reflection/Execution/AssemblyBinderImplementation.cs`. Esse binder percorre `ScopeGroups` (os assemblies cujos metadados foram embutidos), compara nome e, se a referencia especifica versao, `AssemblyVersionMatches(refVersion, defVersion)`; falha com `FileNotFoundException` ou `FileLoadException` (`RefDefMismatch`). Sao as mesmas regras de versao do binder do CoreCLR (arquivo 05: `def >= ref`, componentes -1 como curingas), reimplementadas em C# sobre dados estaticos. **Nao ha busca em disco.**

Consequencia: `Assembly.Load("Foo")` so tem sucesso se `Foo` foi compilado no binario **e** seus metadados sobreviveram ao trimming. A lista de "roots"/preservacao e controlada por `rd.xml` (`src/coreclr/nativeaot/docs/rd-xml-format.md`) e pelos atributos de trimming (`DynamicallyAccessedMembers`, `RequiresUnreferencedCode` / `RequiresDynamicCode`).

### `AssemblyLoadContext` em NativeAOT: um stub

`src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs` (parte do mesmo tipo parcial que o de `src/libraries/System.Private.CoreLib/...`; os trechos dinamicos estao sob `#if !NATIVEAOT` no arquivo comum):

```csharp
internal static Assembly[] GetLoadedAssemblies() => ReflectionAugments.GetLoadedAssemblies();
public Assembly LoadFromAssemblyName(AssemblyName n) => Assembly.Load(n);          // consulta metadados embutidos
private static IntPtr InitializeAssemblyLoadContext(...) => IntPtr.Zero;           // sem binder nativo
private static void PrepareForAssemblyLoadContextRelease(...) { }                  // sem unload
public static AssemblyLoadContext? GetLoadContext(Assembly a) => Default;          // so existe o Default
private static Assembly InternalLoadFromPath(string? p)  { ... throw new PlatformNotSupportedException(); }
internal Assembly InternalLoad(ReadOnlySpan<byte> raw, ReadOnlySpan<byte> sym) { ... throw new PlatformNotSupportedException(); }
public void SetProfileOptimizationRoot(string d) { }  public void StartProfileOptimization(string p) { }   // no-ops
```

Ou seja: `LoadFromAssemblyPath`, `LoadFromStream`, `Assembly.LoadFile/LoadFrom` e variantes lancam `PlatformNotSupportedException`; `Resolving`/`AssemblyResolve`/`TypeResolve`/`ResourceResolve` existem como membros, mas `ReferenceUnreferencedEvents()` apenas os "toca" para que o trimmer os preserve (arquivo 10). `AssemblyDependencyResolver` depende de `hostpolicy` e e inutil aqui. Coletaveis: o construtor aceita `isCollectible`, mas `InitializeAssemblyLoadContext` devolve `IntPtr.Zero` e nao ha descarregamento real (arquivo 08).

### Bibliotecas nativas em NativeAOT

`src/coreclr/nativeaot/System.Private.CoreLib/src/Internal/Runtime/CompilerHelpers/InteropHelpers.cs` (`ResolvePInvokeSlow`, ~linha 257) implementa a resolucao na primeira chamada, via `MethodFixupCell` preenchida pelo compilador com nome da DLL, ponto de entrada e `DllImportSearchPath`:

```
1. NativeLibrary.LoadLibraryCallbackStub(...)  // DllImportResolver registrado (SetDllImportResolver)
2. NativeLibrary.LoadBySearch(..., searchAssemblyDirectory: dllImportSearchPath & AssemblyDirectory, ...)  // busca padrao
3. AssemblyLoadContext.GetLoadContext(calling).GetResolvedUnmanagedDll(...)  // evento ResolvingUnmanagedDll
4. Falha => erro com os motivos agregados
```

Padrao de `DllImportSearchPath` sem atributo: `AssemblyDirectory`. A busca tem sua propria copia em `NativeLibrary.NativeAot.cs` (`LoadBySearch` tambem tenta `Path.Combine(AppContext.BaseDirectory, nome)`). Para evitar toda a busca, `--directpinvoke` / `--directpinvokelist` fazem o ILC emitir chamadas diretas a simbolos linkados no binario final (o P/Invoke vira uma chamada nativa estatica). O equivalente do lado CoreCLR e `PINVOKE_OVERRIDE` (arquivo 09).

### Single-file: no meio do caminho

Single-file (CoreCLR) mantem o runtime dinamico, mas empacota os `.dll` num bundle: o host publica `bundle_probe` no `host_runtime_contract` (`src/native/corehost/host_runtime_contract.h`), o binder tenta o bundle **antes** da TPA (`BindByTpaList`, `AssemblyProbeExtension::Probe`, `src/coreclr/binder/assemblybindercommon.cpp`), assemblies do bundle **nao entram na TPA** (`probe_result_t::bundled` em `deps_resolver.cpp`) e satelites idem. `Assembly.Location` pode ser vazio (`IL3000` e as supressoes "SingleFile" vistas em `AssemblyLoadContext.cs`). Em .NET 10, nativos extraidos do bundle sao indicados por `BUNDLE_EXTRACTION_PATH` (`docs/design/features/host-runtime-information.md`). Ainda assim `LoadFromAssemblyPath` funciona para arquivos externos.

### ReadyToRun e crossgen2

R2R tambem resolve referencias em compile-time (crossgen2 usa `--reference`), mas guarda IL + codigo nativo e o binder em runtime continua o mesmo. `docs/design/features/unloadability.md`: R2R e tratado como IL puro em contextos coletaveis; `assemblyloadcontext.md`: carregar o mesmo R2R de um mesmo caminho em varios contextos aproveita o codigo pre-compilado so na primeira vez. Contraste: NativeAOT nao tem JIT para cair de volta.

### WASM e outros alvos

`AssemblyLoadContext.cs` tem ramos `TARGET_BROWSER` (satelites com caminho virtual `/{cultura}/{nome}.dll`; assemblies via `external_assembly_probe` tem `Location` vazio) e o contrato `external_assembly_probe` em `host_runtime_contract.h` permite ao host fornecer bytes de assemblies sem sistema de arquivos. `src/native/corehost/browserhost` e `wasihost` existem como hosts dedicados; `AssemblyDependencyResolver` e `[UnsupportedOSPlatform("browser")]`.

### Tabela de compatibilidade (API -> comportamento em AOT)

| API | CoreCLR | NativeAOT |
|---|---|---|
| `Assembly.Load(AssemblyName)` | Binder (TPA/ALC) | Metadados embutidos; so assemblies compilados |
| `AssemblyLoadContext.Default` | Contexto TPA | Stub unico |
| `new AssemblyLoadContext(...)` | Contexto novo, opcionalmente coletavel | Cria objeto sem binder (`IntPtr.Zero`); sem carga |
| `LoadFromAssemblyPath/Stream`, `Assembly.LoadFile/LoadFrom` | OK | `PlatformNotSupportedException` |
| `AssemblyDependencyResolver` | OK | Indisponivel na pratica (precisa de hostpolicy) |
| `Resolving`/`AssemblyResolve` | Funcionais | Nao ha binder dinamico para dispara-los |
| `NativeLibrary.SetDllImportResolver` | Funcional | Funcional (`LoadLibraryCallbackStub`) |
| `LoadUnmanagedDll` virtual | Chamado (so nao-Default) | Nao e consultado; somente `ResolvingUnmanagedDll` |
| `Reflection.Emit` | OK | Nao suportado em geral (`RequiresDynamicCode`) |
| `Unload()` | OK (coletavel) | Nao ha descarregamento |

(A ultima linha sobre `Reflection.Emit` e a tradicional restricao de AOT; a base verificada neste repo e o fato de o ALC ser stub e `InternalLoad*` lancar `PlatformNotSupportedException`.)

### Fluxo comparado de uma referencia `Foo` (diagrama)

```
CoreCLR (JIT/R2R)                                         NativeAOT
-----------------                                         ---------
build: gera Foo.dll + deps.json                           build: ILC le -r Foo.dll, resolve, trima, embute
run:   hostpolicy -> TPA {Foo -> C:\app\Foo.dll}          run:   (nada de host/TPA)
       tipo de Foo usado pela 1a vez                             tipo de Foo ja e codigo/simbolo no binario
       binder: cache -> TPA -> carrega PE -> registra            Assembly.Load("Foo"): AssemblyBinderImplementation.Bind
       (falha? Load / Resolving / AssemblyResolve)                 (nao achou nos metadados embutidos? FileNotFoundException)
```

### Perguntas que o modo AOT impoe ao desenho

- **O que a reflexao pode ver?** Somente o que o compilador preservou. `GetLoadedAssemblies()` em AOT vem de `RuntimeAssemblyInfo.GetLoadedAssemblies` (metadados embutidos), nao de uma tabela dinamica de binder.
- **Qual a identidade de um modulo?** O mesmo nome/versao do metadado, comparados por `AssemblyVersionMatches`; chaves publicas e cultura nao viram arquivo.
- **Quem decide o RID?** O compilador (`-r <rid>` no `dotnet publish`, conforme `src/coreclr/nativeaot/docs/compiling.md`); nao ha selecao de assets nativos em runtime.
- **O que acontece com satelites?** O ILC aceita `--satellite` (assemblies satelites associados a entradas/referencias, `ILCompilerRootCommand.cs`); nao verifiquei neste repo o mecanismo exato de lookup em runtime, mas o binder embutido (`AssemblyBinderImplementation`) so enxerga metadados incluidos no binario, sem sondagem de `cultura/nome.dll` no disco.

## Por que

- **Eliminar o custo de resolver e JITar**: ao fechar o mundo na compilacao, startup e footprint caem; nao precisa de host, `deps.json`, binder nem JIT.
- **Trimming exige mundo fechado**: so e seguro remover codigo se o compilador souber **todos** os modulos e usos; carga dinamica arbitraria quebra a premissa. Por isso o ALC vira stub e as APIs de carga lancam.
- **Reaproveitar a semantica de versao** no binder embutido (`AssemblyVersionMatches`) preserva compatibilidade: um `Assembly.Load("Foo, Version=1.0.0.0")` se comporta igual a CoreCLR quando o assembly existe.
- **Direct P/Invoke** substitui `dlopen` em runtime por linkagem estatica: mais rapido, sem superficie de planting.
- **Single-file/bundle** e uma concessao: mantem o runtime dinamico (plugins externos) com distribuicao simples.
- **Manter a API compilavel** (os membros existem e lancam) permite que bibliotecas compartilhadas compilem para ambos os modos, falhando apenas se o caminho dinamico for exercitado.

## Trade-offs

- **Perde-se extensibilidade em runtime**: sem plugins, sem hot reload, sem `Reflection.Emit`; ecossistema de bibliotecas precisa marcar `RequiresDynamicCode`/`RequiresUnreferencedCode` e fornecer geradores de codigo (source generators).
- **Erros so em runtime para caminhos dinamicos**: `PlatformNotSupportedException` e falhas de `Type.GetType` por trimming aparecem tarde; os analisadores de trimming (`IL2026`, `IL3050`...) tentam mover isso para build.
- **Metadados parciais**: reflexao em AOT depende de `rd.xml`/atributos; erros de "type not found" sao comuns.
- **Mais um modo de teste**: cada API de carga precisa de duas semanticas e duas suites.
- **Bibliotecas nativas**: sem `LoadUnmanagedDll` virtual, plugins nativos dependem de `DllImportResolver`; `directpinvoke` obriga a conhecer o binario final.
- **Tamanho/tempo de compilacao**: o ILC deve processar todo o grafo; `--multifile` reduz mas fragmenta a otimizacao.
- **Versionamento**: em AOT nao ha "roll-forward" de framework; atualizar o runtime exige recompilar.

## Como aplicar na sua linguagem

1. **Decida cedo se o mundo e aberto ou fechado** e modele isso como *capability* do alvo: `dynamic_load: yes/no`. Faca todas as APIs de carga dinamica checarem essa capability e falharem com um erro especifico (`NotSupportedInThisMode`), como `PlatformNotSupportedException` do .NET.
2. **Separe a interface do binder da implementacao**: uma interface `ModuleBinder.bind(ref) -> Module | Error` com (a) implementacao "arquivo + manifesto" e (b) implementacao "tabela embutida" (`AssemblyBinderImplementation`). O resto do runtime nao deve saber qual esta ativa.
3. **Use a mesma funcao de compatibilidade de versao** nos dois binders (extraia-a para um modulo unico e teste-a por tabela); no .NET ela foi duplicada em C++ e em C#, risco de divergencia.
4. **Mova a resolucao de arquivos para o compilador** no modo AOT: aceite `--reference` e `--satellite` equivalentes, produza um manifesto embutido e registre quais modulos tem metadados de reflexao.
5. **Ofereca um mecanismo declarativo de preservacao** (`rd.xml` equivalente + atributos no codigo) e analisadores que alertem em build quando algo dinamico usa um modulo que pode ter sido removido.
6. **Forneca "direct native call"** (lista de simbolos nativos linkados estaticamente) alem de uma busca dinamica por nome; no AOT use por padrao.
7. **Mantenha os membros de API presentes mesmo quando nao suportados**, para evitar fragmentar o ecossistema; documente tabela de compatibilidade (como a acima).
8. **Se oferecer modo intermediario** (single-file/bundle), implemente "probe de bundle antes do mapa da plataforma" e nao ponha itens do bundle no mapa; mantenha `path = null` como valor valido.
9. **Teste a matriz**: {dinamico, bundle, AOT} x {Default, plugin, nativo, satelite}, usando como modelo `src/tests/Loader/` e `src/libraries/System.Runtime.Loader/tests/`.
10. **Considere WASM/host sem filesystem** desde o inicio: um contrato de "probe externo" (`external_assembly_probe`) onde o host entrega bytes por nome resolve embarcado, navegador e testes.

## Referencias

- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/Internal/Reflection/Augments/ReflectionAugments.cs` (`Load`)
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Reflection/Runtime/General/Dispensers.cs`, `.../Assemblies/RuntimeAssemblyInfo.cs`
- `src/coreclr/nativeaot/System.Private.TypeLoader/src/Internal/Reflection/Execution/AssemblyBinderImplementation.cs`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/Internal/Runtime/CompilerHelpers/InteropHelpers.cs`, `.../InteropServices/NativeLibrary.NativeAot.cs`
- `src/coreclr/tools/aot/ILCompiler/ILCompilerRootCommand.cs`; `src/coreclr/nativeaot/docs/` (`compiling.md`, `rd-xml-format.md`, `optimizing.md`, `troubleshooting.md`)
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` (blocos `#if !NATIVEAOT`, `TARGET_BROWSER`)
- `src/native/corehost/host_runtime_contract.h`, `src/coreclr/binder/assemblybindercommon.cpp` (`AssemblyProbeExtension`), `src/native/corehost/hostpolicy/deps_resolver.cpp` (`bundled`)
- `docs/design/features/host-runtime-information.md`, `unloadability.md`, `assemblyloadcontext.md`
- ECMA-335 Partition II 6 (Assemblies) e Partition I 8 (Common Type System: module/assembly): a especificacao nao exige carga dinamica; AOT e uma estrategia de implementacao valida.
