# Lazy loading e o evento Resolving/AssemblyResolve

> Objetivo: descrever quando um assembly e de fato carregado (preguicosamente, no primeiro uso), quais ganchos o codigo do usuario tem para fornecer um assembly que o binder nao achou (`AssemblyLoadContext.Resolving`, `AppDomain.AssemblyResolve`, mais `TypeResolve`/`ResourceResolve`) e como esses ganchos se encaixam na ordem de resolucao.

## Mecanismo no .NET

### Carga sob demanda

O .NET nao carrega o fecho transitivo de referencias no startup. `docs/design/features/assemblyloadcontext.md` descreve tres origens de carga: explicita (`Assembly.Load`), **implicita ("resolving static assembly references from the manifest")** e dinamica (emissao). A forma implicita ocorre quando o runtime precisa de um tipo/metodo do assembly referenciado, tipicamente ao carregar um tipo ou ao compilar (JIT) um metodo que o usa. Ate la, a referencia `AssemblyRef` e so uma linha em metadados. Isso e a base do *lazy loading* do .NET: **a carga e consequencia do primeiro uso**, o que tem efeitos diretos: (a) startup rapido, (b) erros de "assembly nao encontrado" surgem tarde, dentro de `FileNotFoundException`/`FileLoadException` em meio a execucao, (c) plugins podem fornecer dependencias "na hora" por callbacks.

(Os eventos `AssemblyLoad` de `AppDomain` e o `BinderTracing` permitem observar exatamente quando cada carga acontece; ver `InvokeAssemblyLoadEvent`/`OnAssemblyLoad` em `AssemblyLoadContext.cs`.)

### Os ganchos, na ordem em que sao consultados

```
bind nativo (cache do contexto, TPA, app paths, bundle)      <- arquivos 03/04
   |  falhou: FILE_NOT_FOUND | APP_DOMAIN_LOCKED | REF_DEF_MISMATCH
   v
1. ALC.Load(AssemblyName)                      (override virtual; Default: sempre null)
2. fallback ao Default binder                  (so Custom, nao-satelite)
3. ALC.ResolveSatelliteAssembly                (so satelites)
4. ALC.Resolving event                         <- ultimo recurso POR CONTEXTO
   |  nada resolveu
   v
5. AppDomain.AssemblyResolve event             <- ultimo recurso GLOBAL (AppDomain::TryResolveAssemblyUsingEvent)
   |  nada resolveu
   v
FileNotFoundException / FileLoadException
```

As etapas 1-4 sao implementadas por `RuntimeInvokeHostAssemblyResolver` (`src/coreclr/vm/appdomain.cpp`, ~linha 3898), cujos comentarios numeram os "Step 2/3/4/5 of CustomAssemblyBinder::BindAssemblyByName", com tracing por estagio (`BinderTracing::ResolutionAttemptedOperation::Stage::AssemblyLoadContextLoad`, `ResolveSatelliteAssembly`, `AssemblyLoadContextResolvingEvent`). A etapa 5 e `AppDomain::TryResolveAssemblyUsingEvent` (~linha 3164), que chama `RaiseAssemblyResolveEvent` (~linha 3571) -> `AssemblyLoadContext.OnAssemblyResolve` gerenciado (`METHOD__ASSEMBLYLOADCONTEXT__ON_ASSEMBLY_RESOLVE` em `corelib.h`). Ela nao roda para binds por codebase (`pSpec->GetName() == nullptr`), e excecoes nao transitorias do handler viram `EEFileLoadException` e sao colocadas no cache de falhas.

### `AssemblyLoadContext.Resolving`

`AssemblyLoadContext.cs` (~linha 205): "This event is raised if the managed assembly could not be resolved via the default resolution logic [including AssemblyLoadContext.Load()]". Assinatura: `event Func<AssemblyLoadContext, AssemblyName, Assembly?>? Resolving`. Implementacao (`GetFirstResolvedAssemblyFromResolvingEvent`): percorre a lista de invocacao (`Delegate.EnumerateInvocationList`) e **retorna o primeiro resultado nao nulo**; nao ha agregacao. Cada chamada pode ser traceada (`TraceResolvingHandlerInvoked`). O resultado passa por `ValidateAssemblyNameWithSimpleName` (tem de ser `RuntimeAssembly` e ter o mesmo nome simples, case-insensitive; `InvalidOperationException` caso contrario). O handler recebe o **contexto** que iniciou o bind, entao uma unica rotina pode servir varios contextos.

Exemplo canonico do repo (`docs/design/features/host-startup-hook.md`, uma "GAC-like" para o .NET Core): um startup hook inscreve um resolvedor no Default.

```csharp
internal class StartupHook
{
    public static void Initialize()
        => AssemblyLoadContext.Default.Resolving += SharedAssemblyResolver.LoadAssemblyFromSharedLocation;
}
static Assembly? LoadAssemblyFromSharedLocation(AssemblyLoadContext ctx, AssemblyName name)
{
    string? path = FindInSharedLocation(name);
    return path is null ? null : AssemblyLoadContext.Default.LoadFromAssemblyPath(path);
}
```

O doc dessa feature diz que isso serve para resolver dependencias **fora da TPA** de um local compartilhado, "similar to the GAC on .NET Framework".

### `AppDomain.AssemblyResolve` (global)

Em `AssemblyLoadContext.cs`: `internal static event ResolveEventHandler? AssemblyResolve` (exposto publicamente por `AppDomain`), ao lado de `TypeResolve`, `ResourceResolve` e `AssemblyLoad`. `InvokeResolveEvent` cria um `ResolveEventArgs(name, requestingAssembly)` e percorre os handlers, devolvendo o primeiro `RuntimeAssembly` nao nulo (`GetRuntimeAssembly(asm)`; assembly dinamico/Builder e ignorado). Diferencas importantes frente a `Resolving`:

| Aspecto | `ALC.Resolving` | `AppDomain.AssemblyResolve` |
|---|---|---|
| Escopo | por contexto | processo inteiro (todos os contextos) |
| Argumento | `AssemblyName` estruturado | `ResolveEventArgs.Name` (string, display name) + `RequestingAssembly` |
| Ordem | antes | depois de `Resolving` |
| Quando dispara | binder de um ALC nao resolveu | ainda nao resolvido apos `Load`/`Resolving` |
| Uso recomendado | resolver dependencias de um contexto | compat. legada; casos globais |

Um uso real dentro do proprio framework: `Assembly.LoadFrom` (`src/libraries/System.Private.CoreLib/src/System/Reflection/Assembly.cs`, ~linhas 305-403). Na primeira chamada registra `AssemblyLoadContext.AssemblyResolve += LoadFromResolveHandler`; mantem `s_loadFromAssemblyList`; o handler so atua se o `RequestingAssembly` esta no Default **e** foi carregado por `LoadFrom`; entao tenta `<dir do requisitante>/<nome>.dll` e chama `LoadFrom` nele. Isso reproduz o antigo contexto `LoadFrom` do .NET Framework (resolver dependencias na mesma pasta) *sem* um contexto novo: dentro do Default, via evento. Ha `TraceAssemblyLoadFromResolveHandlerInvoked` para diagnostico e um `File.Exists` previo para evitar excecao de primeira chance (exceto em BROWSER/WASI, onde o arquivo pode estar embutido).

### `TypeResolve` e `ResourceResolve`

Mesmo mecanismo (`InvokeResolveEvent`) para: (a) tipo nao encontrado em um assembly (tipicamente Reflection.Emit com tipos ainda nao criados); (b) recurso do manifesto nao encontrado. Ambos tem QCalls `OnTypeResolve`/`OnResourceResolve` (`[UnmanagedCallersOnly]`). Testes: `src/tests/Loader/ResourceResolve/`, `src/libraries/System.Runtime.Loader/tests/ResourceAssemblyLoadContext.cs`.

### Trimming/AOT: eventos que precisam ser preservados

`AssemblyLoadContext.NativeAot.cs` tem `ReferenceUnreferencedEvents()` que toca `AssemblyLoad`, `ResourceResolve`, `_resolving`, `TypeResolve`, `AssemblyResolve` para o compilador nao os remover: o runtime nativo ainda os espera mesmo sem ninguem inscrito. Em NativeAOT, porem, nao ha binder dinamico para dispara-los na pratica (arquivo 12).

### Carga explicita sem lazy: `Assembly.Load`, `Type.GetType`

APIs contextuais (`Assembly.Load(string)`, `Activator.CreateInstance(assemblyName, typeName)`, `Type.GetType("A.B, Asm")`) carregam *no ato* e escolhem o contexto pelo chamador (ou por `CurrentContextualReflectionContext`, `docs/design/features/AssemblyLoadContext.ContextualReflection.md`). Para programas com plugins, o par "lazy + contexto inferido" e a principal fonte de bugs.

### Padroes praticos

```csharp
// 1. Compartilhar dependencias do host com plugins: responder no Resolving do contexto do plugin
pluginAlc.Resolving += (ctx, name) =>
    hostAssemblies.TryGetValue(name.Name!, out var asm) ? asm : null;

// 2. Carregar sob demanda de um repositorio (ex.: banco, rede, recurso embutido)
AssemblyLoadContext.Default.Resolving += (ctx, name) =>
{
    using Stream? s = Repository.Open(name.Name + ".dll");
    return s is null ? null : ctx.LoadFromStream(s);
};

// 3. Diagnostico: enxergar o que ninguem resolveu
AppDomain.CurrentDomain.AssemblyResolve += (_, e) =>
{
    Console.Error.WriteLine($"nao resolvido: {e.Name} (pedido por {e.RequestingAssembly?.FullName})");
    return null;                               // null = continue a cadeia
};
```

### Resumo de quem dispara o que

| Evento / callback | Nivel | Definido em | Invocado por |
|---|---|---|---|
| `ALC.Load` (virtual) | por contexto | `AssemblyLoadContext.cs` | `RuntimeInvokeHostAssemblyResolver` (passo 2) |
| `ALC.ResolveSatelliteAssembly` | por contexto | `AssemblyLoadContext.cs` | idem (passo 4, so satelites) |
| `ALC.Resolving` | por contexto | `AssemblyLoadContext.cs` | idem (passo 5) |
| `AppDomain.AssemblyResolve` | global | `AssemblyLoadContext.cs` (estatico) | `AppDomain::TryResolveAssemblyUsingEvent` |
| `AppDomain.TypeResolve` / `ResourceResolve` | global | idem | VM ao falhar tipo / recurso |
| `AppDomain.AssemblyLoad` | global (notificacao) | idem | `InvokeAssemblyLoadEvent` apos carga |
| `ALC.ResolvingUnmanagedDll` | por contexto | idem | arquivo 09 |

## Por que

- **Lazy por padrao**: apps grandes tem milhares de referencias, a maioria nunca exercitada num dado run; carregar sob demanda mantem startup e memoria baixos.
- **Eventos como "ultimo recurso"** permitem estender o resolvedor sem herdar `AssemblyLoadContext`, util para libs que nao controlam a criacao do contexto (startup hooks, profilers, frameworks de teste).
- **`Resolving` por contexto + `AssemblyResolve` global**: o primeiro e a interface moderna e escopada; o segundo foi mantido por compatibilidade com o .NET Framework (muito codigo existente depende de `AppDomain.AssemblyResolve`) e porque serve a `LoadFrom`.
- **Evento depois do binder (nao antes)**: preserva a regra "o que a plataforma sabe resolver nao pode ser sequestrado" (para `Resolving`) e mantem o caminho feliz sem callbacks.
- **Primeiro resultado nao nulo vence**: semantica simples; evita conflito de merge de varios handlers.
- **Validacao de nome do resultado** evita que um handler devolva assembly diferente do pedido.

## Trade-offs

- **Falhas tardias e nao locais**: o erro surge onde o tipo e usado pela primeira vez, possivelmente em caminho raro de producao; difícil de testar sem exercitar todos os caminhos (por isso ferramentas como `ILVerify`/analisadores e pre-carregamento).
- **Reentrancia e deadlock**: handlers que carregam assemblies disparam novos binds; o runtime eleva limites de nivel de carga para permitir isso (`OVERRIDE_LOAD_LEVEL_LIMIT` em `RaiseAssemblyResolveEvent`), mas recursao ainda lanca excecao.
- **Estado global** (`AssemblyResolve`): afeta todos os contextos e e dificil de remover; handlers esquecidos impedem descarregamento (arquivo 08).
- **Ordem entre handlers** e a de inscricao; bibliotecas diferentes podem conflitar.
- **Desempenho**: cada miss pode custar IO + chamada gerenciada; o cache de falhas mitiga.
- **Seguranca**: um handler pode servir qualquer assembly; combinar com `AssemblyLoadContext.Load` do plugin torna a politica menos visivel.
- **Inconsistencia de contexto**: `ctx.LoadFromStream` dentro de `Resolving` carrega no contexto correto, mas `Assembly.Load` dentro do handler carrega no contexto do chamador, nao no `ctx`.

## Como aplicar na sua linguagem

1. **Decida a estrategia de carga**: *eager* (resolve tudo no inicio; falhas cedo; startup lento) ou *lazy* (primeiro uso). Se lazy, ofereca uma operacao `verify_imports()`/`preload()` para quem quiser falhar cedo, e um modo de diagnostico que liste imports nao resolvidos de um modulo sem executa-lo.
2. **Tenha exatamente um gancho moderno e escopado** (`on_resolve(scope, name)`) e **evite criar um segundo global**. Se precisar de compatibilidade, deixe-o explicitamente depois do escopado, como o .NET.
3. **Primeiro nao-nulo vence**, e documente a ordem de handlers. Valide o nome devolvido contra o pedido.
4. **Defina a posicao dos hooks**: depois do mapa da plataforma e antes do erro final; nunca antes da plataforma.
5. **Passe informacao suficiente ao hook**: nome estruturado, versao pedida, requisitante e escopo (o `AssemblyLoadContext.Resolving` passa o contexto; `AssemblyResolve` passa `RequestingAssembly`).
6. **Trace cada estagio** (qual hook respondeu, em qual ordem, com qual resultado) -- os "estagios" do `BinderTracing` pagam o custo de implementacao na primeira investigacao.
7. **Previna recursao**: um limite de profundidade ou um conjunto de "bind em andamento" por (escopo, nome); retorne erro claro em vez de deadlock.
8. **Armazene falhas em cache com cuidado**: nao cacheie falhas transitorias (`FILE_NOT_FOUND` sob `skipFailureCaching`); cacheie falhas definitivas (arquitetura invalida, versao incompativel).
9. **Se linguagem tem AOT**, deixe claro que hooks dinamicos so valem em modo dinamico (arquivo 12), e faca o linker preservar os pontos de entrada necessarios.
10. **Para plugins**, ofereca um helper pronto (`ComponentResolver`) que ja se encaixe em `on_resolve` e `load`, para que ninguem escreva a regra "se e do host, devolva o do host" a mao.

## Referencias

- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` (`Resolving`, `AssemblyResolve`, `TypeResolve`, `ResourceResolve`, `InvokeResolveEvent`, `GetFirstResolvedAssemblyFromResolvingEvent`, `ValidateAssemblyNameWithSimpleName`)
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.CoreCLR.cs` (`Trace*`)
- `src/coreclr/vm/appdomain.cpp` (`RuntimeInvokeHostAssemblyResolver`, `TryResolveAssemblyUsingEvent`, `RaiseAssemblyResolveEvent`), `src/coreclr/vm/corelib.h`
- `src/coreclr/binder/assemblybindercommon.cpp` (`BindUsingHostAssemblyResolver`), `bindertracing.cpp`, `customassemblybinder.cpp`
- `src/libraries/System.Private.CoreLib/src/System/Reflection/Assembly.cs` (`LoadFrom`, `LoadFromResolveHandler`)
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs`
- `docs/design/features/assemblyloadcontext.md`, `host-startup-hook.md`, `AssemblyLoadContext.ContextualReflection.md`
- `src/tests/Loader/ResourceResolve/`, `src/libraries/System.Runtime.Loader/tests/`
- ECMA-335 Partition II secao 6 (Assemblies, referencias a assemblies): a CLI deixa a localizacao de assemblies como "implementation-specific"; os eventos sao extensoes do .NET.
