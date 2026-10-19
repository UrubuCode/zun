# Collectible/unloadable contexts (descarregamento)

> Objetivo: explicar como o .NET descarrega modulos sem AppDomains: a unidade de memoria (`LoaderAllocator`), o protocolo de duas fases, o que o impede e o que nao e suportado. Fonte principal: `docs/design/features/unloadability.md` (com o diagrama `unloadability.svg`) e o codigo em `src/coreclr/vm/loaderallocator.*`.

## Mecanismo no .NET

### API

Em `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs`:

```csharp
protected AssemblyLoadContext(bool isCollectible)            // linha ~75
public    AssemblyLoadContext(string? name, bool isCollectible = false)
public bool IsCollectible => _isCollectible;
public event Action<AssemblyLoadContext>? Unloading;         // linha ~226
public void Unload();                                        // linha ~499
```

`Unload()` lanca `InvalidOperationException` se `!IsCollectible`; senao chama `GC.SuppressFinalize(this)` e `InitiateUnload()`. O finalizador `~AssemblyLoadContext()` tambem chama `InitiateUnload()` (so para coletaveis; nos nao coletaveis o finalizador e suprimido no construtor com `GC.SuppressFinalize`). Como `Unload()` e idempotente em estado (`_state != Alive` nao refaz o trabalho), chamar duas vezes e permitido (teste `DoubleUnload_CollectibleWithNoAssemblyLoaded` em `src/libraries/System.Runtime.Loader/tests/CollectibleAssemblyLoadContextTest.cs`). Depois de `Unload()`, qualquer `LoadFromAssemblyPath` falha com `InvalidOperationException` (mesmo teste, `VerifyIsAlive`).

Tambem existem `Assembly.IsCollectible` e `MemberInfo.IsCollectible` para o codigo se proteger de cachear tipos de contextos coletaveis.

### Unidade de posse: `LoaderAllocator`

O descarregamento se apoia em "collectible types" que ja existiam. Para cada ALC coletavel cria-se um `AssemblyLoaderAllocator` (`src/coreclr/vm/loaderallocator.hpp`, classe `AssemblyLoaderAllocator : public LoaderAllocator`, linha ~995), de onde sai **toda** a memoria especifica do contexto: dados de tipos (`MethodTable`, `EEClass`), codigo JITado, stubs/thunks, o `AssemblySpecBindingCache` do contexto e as listas de `DomainAssembly`. Campos relevantes: `m_fUnloaded`, `m_fTerminated`, `m_fGCPressure`; funcoes `Destroy`, `GCLoaderAllocators`, `GCLoaderAllocators_RemoveAssemblies`.

Do lado gerenciado existem os pares `LoaderAllocator` e `LoaderAllocatorScout`: a instancia de `LoaderAllocator` e alcancavel a partir dos `MethodTable` de objetos do contexto (setas tracejadas do diagrama); o `Scout` tem um finalizador que dispara a destruicao nativa.

### Protocolo em duas fases (`unloadability.md`)

**Fase 1 (iniciada por `Unload()` ou pelo finalizador do ALC):**

1. Dispara o evento `Unloading` (limpeza do usuario: parar threads, remover referencias, liberar handles). `RaiseUnloadEvent` usa `Interlocked.Exchange(ref _unloading, null)` para disparar so uma vez. Em `OnProcessExit` o evento tambem e disparado para todos os contextos vivos.
2. `InitiateUnload()` (sob `_unloadLock`, transicao `Alive -> Unloading`) cria um `GCHandle` **forte** para o proprio ALC e chama `PrepareForAssemblyLoadContextRelease` (QCall `AssemblyNative_PrepareForAssemblyLoadContextRelease`). Motivo: finalizadores de tipos do contexto podem precisar do ALC.
3. No nativo, `CustomAssemblyBinder::PrepareForLoadContextRelease` guarda o handle forte em `m_ptrManagedStrongAssemblyLoadContext`, decrementa a contagem de referencia do `AssemblyLoaderAllocator` e destroi o handle forte para o `LoaderAllocator` gerenciado -- agora ele pode ser coletado. O ALC e removido do registro global `AllContexts`.

**Fase 2 (quando o GC prova que nenhuma instancia/tipo/assembly do contexto esta viva):**

1. O GC coleta o `LoaderAllocator` gerenciado e, em seguida, o `LoaderAllocatorScout`; o finalizador deste chama `LoaderAllocator::Destroy` nativo.
2. `Destroy` decrementa as contagens de refs; se nao e a ultima, o alocador nativo fica vivo (outros alocadores o referenciam) e sera destruido depois.
3. Na ultima referencia, `LoaderAllocator::GCLoaderAllocators` limpa os `DomainAssembly` (remove do AppDomain, do cache de bind, notifica o depurador, destroi), libera os handles forte/fraco longo para o ALC gerenciado (permitindo coleta do proprio ALC).
4. A destruicao efetiva ocorre na thread finalizadora (`Thread::DoExtraWorkForFinalizer`) e destroi tambem o `CustomAssemblyBinder`.

Consequencia: **descarregar e cooperativo e assincrono**. Nao ha abort de threads; o contexto so some quando nenhum codigo dele esta na pilha e nenhuma referencia externa chega aos seus tipos/objetos.

### Trabalho necessario para suportar features nos coletaveis

O doc lista o que teve de ser adaptado (cada item e uma fonte de bugs de unload):

- **Interop (P/Invoke)**: bastou remover a checagem que proibia.
- **Thread statics**: arrays de `object[]` de statics por thread passam a ser mantidos por handles alocados do `LoaderAllocator` (e nao handles globais), liberados quando a thread termina; `ThreadLocalModule` destruidos junto com o alocador. Foi preciso permitir liberar handles individuais.
- **Marshalling de delegates** de tipos coletaveis: so remover checagens (os thunks nativo->gerenciado vem de um alocador global e sao liberados quando o delegate e coletado).
- **COM interop**: `ComCallableWrapperCache` migrou do AppDomain para o `LoaderAllocator`.
- **`FixedAddressValueTypeAttribute`**: desde o .NET 9 esses campos vivem no Pinned Object Heap (nao sao mais presos por um GCHandle que impediria a coleta).
- Thunks de codigo (tail call arg copy, shuffle thunk cache, JIT helper logging) e unwind info passaram a ser alocados do alocador correto.

### Nao suportado em ALCs coletaveis (decidido no doc)

- Carregar assemblies **IJW** (C++/CLI misto).
- Usar codigo **ReadyToRun**: assemblies R2R sao carregados como IL puro (precisam de JIT).
- Unload "forcado" por abort de threads (nao-objetivo).

(A secao "Constraints" de `assemblyloadcontext.md` ainda diz que contextos custom "nao podem ser descarregados"; o texto e anterior ao feature. `unloadability.md` e a referencia atual.)

### O que normalmente impede o descarregamento

Todos derivam da regra "alguma raiz forte alcanca o contexto":

- Referencia estatica, em outro contexto, a objeto/tipo/delegate do plugin.
- Thread em execucao que ainda tem frame do plugin, ou thread de pool/timer criada pelo plugin.
- `GCHandle` forte, `ThreadStatic` em thread que ainda vive (mitigado acima), `AsyncLocal`, `ConditionalWeakTable` com chave do plugin em estrutura do host.
- Eventos do host com handler do plugin (`AppDomain.AssemblyResolve`, `Console.CancelKeyPress`, ...): o delegate mantem o `MethodTable` vivo.
- Caches do framework com `Type` do plugin (serializacao, DI, `Enum` cache).
- Finalizadores pendentes (o ALC e mantido vivo com handle forte durante a fase 1).

Diagnostico tipico: `WeakReference` para o ALC, `Unload()`, e laco de `GC.Collect(); GC.WaitForPendingFinalizers();` ate `IsAlive == false`, como nos testes `CollectibleChecker.GcAndCheck()`.

```csharp
[MethodImpl(MethodImplOptions.NoInlining)]          // evita que o JIT mantenha o ALC vivo na pilha
static WeakReference Run(string path)
{
    var alc = new PluginContext(path, isCollectible: true);
    try
    {
        var asm = alc.LoadFromAssemblyPath(path);
        asm.GetType("Plugin.Main")!.GetMethod("Run")!.Invoke(null, null);
        return new WeakReference(alc, trackResurrection: true);
    }
    finally { alc.Unload(); }
}

var wr = Run(@"C:\plugins\a\Plugin.dll");
for (int i = 0; wr.IsAlive && i < 20; i++) { GC.Collect(); GC.WaitForPendingFinalizers(); }
Console.WriteLine(wr.IsAlive ? "vazou" : "descarregado");
```

### Cenarios reais analisados no doc

Roslyn (um contexto por teste; no futuro analyzers em ALC por compilacao), ASP.NET Core (`dotnet watch`, compilacao de Razor), LINQPad (drivers/consultas isoladas), Prism (descoberta de modulos). Conclusao do doc: AppDomains davam "falsa sensacao de seguranca" e nao ajudavam em performance; unload de ALC cobre o caso "carregar, usar, descartar".

### Em outros runtimes

`AssemblyLoadContext.cs` tem comentarios de sincronia com o Mono (`MonoAssemblyLoadContextInternalState`), mas os testes `CollectibleAssemblyLoadContextTest.cs` estao marcados `[ActiveIssue(".../34072", TestRuntimes.Mono)]` e para WASM: o suporte la e parcial. Em NativeAOT, `InitializeAssemblyLoadContext` e `PrepareForAssemblyLoadContextRelease` sao stubs vazios (`AssemblyLoadContext.NativeAot.cs`): nao ha o que descarregar.

### Linha do tempo resumida

```
t0  new ALC(isCollectible: true)        -> binder nativo + AssemblyLoaderAllocator criados; weak handle (WeakTrackResurrection)
t1  LoadFromAssemblyPath(...)           -> DomainAssembly + tipos alocados no LoaderAllocator
t2  alc.Unload()                        -> evento Unloading; Alive->Unloading; handle fraco vira forte; alocador gerenciado solto
t3  GC (gen2) + finalizacao             -> LoaderAllocator/Scout coletados -> LoaderAllocator::Destroy nativo
t4  thread finalizadora                 -> DomainAssemblies e CustomAssemblyBinder destruidos; ALC finalmente coletavel
```

Entre t2 e t4 nada impede o codigo de continuar rodando nas threads que ja estavam dentro do plugin; o contexto so e coletado quando elas saem.

## Por que

- **Substituir AppDomain.Unload** com semantica mais simples: sem abort de thread, sem marshaling entre dominios. O custo: o desenvolvedor precisa garantir ausencia de referencias.
- **Memoria agrupada por contexto** (`LoaderAllocator`) permite liberar tudo de uma vez; sem isso seria necessario rastrear milhares de estruturas individuais.
- **Coleta guiada pelo GC**: reaproveita a correcao do GC (alcancabilidade) em vez de um contador de referencias manual; o custo e a latencia (nao e imediato).
- **Duas fases** resolvem finalizadores e ressurreicao: o ALC fica vivo por um handle forte ate a destruicao nativa.
- **Evento `Unloading`** da ao codigo do usuario uma chance explicita de soltar raizes antes do GC.
- **Limitar features** (IJW, R2R): o doc diz apenas que, "after investigating all the details", decidiu-se nao suportar esses cenarios sem forte demanda. A razao tecnica nao e detalhada no doc; a leitura plausivel (inferencia, nao confirmada no texto) e que codigo pre-compilado e codigo nativo misto criam referencias nativas dificeis de rastrear pelo `LoaderAllocator`.

## Trade-offs

- **Nao deterministico**: `Unload()` apenas inicia; a liberacao real depende de GC e finalizadores.
- **Vazamentos silenciosos**: uma unica referencia perdida mantem o contexto inteiro vivo (e todos os seus assemblies, JIT code, statics).
- **Perda de desempenho**: sem R2R, o plugin coletavel e JITado; thread statics e statics tem indirecoes extras; `LoaderAllocator` tem contabilidade adicional.
- **Complexidade do runtime**: cada feature nova (ex.: `FixedAddressValueType`, thread statics) precisa ser auditada para coletaveis; o doc mostra que varios itens exigiram redesign.
- **Superficie de API**: `IsCollectible` em `Assembly`/`MemberInfo` obriga bibliotecas a evitar caches fortes de tipos.
- **Compatibilidade**: nem todos os runtimes (Mono/WASM) tem a mesma garantia.

## Como aplicar na sua linguagem

1. **Decida se descarregar e requisito.** Se for, projete desde o inicio uma **unidade de posse** (arena/region) por escopo de carga que guarde codigo, tipos, estaticos e caches; e muito mais barato do que retrofitar.
2. **Prefira unload cooperativo e assincrono**, guiado pela alcancabilidade do GC, com um evento/callback `on_unload` para o usuario soltar raizes. Nao implemente "kill threads".
3. **Mantenha o escopo vivo por handle forte durante a destruicao** para finalizadores; separe "iniciar" de "concluir" (maquina de estados `Alive -> Unloading -> Unloaded`).
4. **Documente as raizes classicas** (statics externos, threads, handlers de eventos globais, caches) e forneca uma ferramenta de diagnostico (`runtime.who_holds(scope)` ou dump de caminhos de raiz).
5. **Impeca no tipo** o que nao e coletavel: cache de tipos em estruturas globais devem usar referencias fracas ou chaves por escopo (`Type.is_collectible`).
6. **Declare o que nao funciona em escopos descarregaveis** (por exemplo, codigo pre-compilado AOT, extensoes nativas com enderecos fixos) e imponha isso na carga, como o doc faz para R2R/IJW.
7. **Teste com um harness** estilo `CollectibleChecker`: cria escopo em funcao `NoInlining`, `WeakReference`, laco de GC, asserts de que o evento `Unloading` disparou e de que o escopo foi liberado.
8. **Se sua linguagem e AOT-first**, ofereca descarregamento apenas para plugins em modo interpretado/JIT ou via processos separados.

## Referencias

- `docs/design/features/unloadability.md`, `unloadability.svg`
- `docs/design/features/assemblyloadcontext.md` (secao Constraints, desatualizada quanto a unload)
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` (`Unload`, `InitiateUnload`, `RaiseUnloadEvent`, `OnProcessExit`, finalizador)
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.CoreCLR.cs` (QCalls)
- `src/coreclr/vm/loaderallocator.hpp`, `loaderallocator.cpp`, `assemblynative.cpp`
- `src/coreclr/binder/customassemblybinder.cpp`
- `src/libraries/System.Runtime.Loader/tests/CollectibleAssemblyLoadContextTest.cs`, `src/tests/Loader/CollectibleAssemblies/`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs` (stubs)
- ECMA-335 nao define descarregamento; ele e especifico de implementacao (AppDomain.Unload e ALC.Unload).
