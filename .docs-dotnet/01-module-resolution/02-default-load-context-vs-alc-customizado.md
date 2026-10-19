# Default load context vs ALC customizado

> Objetivo: contrastar o contexto "TPA" (Default), criado pelo runtime no startup, com contextos derivados pelo usuario; mostrar o que cada um pode e nao pode fazer.

## Mecanismo no .NET

### O Default e especial e estatico

Em `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs`:

```csharp
public static AssemblyLoadContext Default => DefaultAssemblyLoadContext.s_loadContext;
...
internal sealed class DefaultAssemblyLoadContext : AssemblyLoadContext
{
    internal static readonly AssemblyLoadContext s_loadContext = new DefaultAssemblyLoadContext();
    internal DefaultAssemblyLoadContext() : base(true, false, "Default") { }
}

internal sealed class IndividualAssemblyLoadContext : AssemblyLoadContext
{
    internal IndividualAssemblyLoadContext(string name) : base(false, false, name) { }
}
```

O primeiro argumento do construtor privado, `representsTPALoadContext`, e `true` apenas para o Default. O VM usa isso para escolher `DefaultAssemblyBinder` em vez de `CustomAssemblyBinder`. `IndividualAssemblyLoadContext` e o contexto anonimo criado por `Assembly.LoadFile` e `Assembly.Load(byte[])` (`assemblyloadcontext.md`).

O construtor publico `AssemblyLoadContext(string? name, bool isCollectible = false)` e o `protected AssemblyLoadContext()` sempre passam `representsTPALoadContext: false`. Ninguem fora do runtime consegue criar um segundo "contexto TPA".

### Diferencas de comportamento (resumo verificavel)

| Aspecto | Default | Customizado |
|---|---|---|
| Binder nativo | `DefaultAssemblyBinder` (`src/coreclr/binder/defaultassemblybinder.cpp`) | `CustomAssemblyBinder` (`customassemblybinder.cpp`) |
| Fonte da lista de assemblies | TPA + resource roots + APP_PATHS vindos do host (`ApplicationContext::SetupBindingPaths`, `applicationcontext.cpp`) | Nenhuma lista previa; so o que o usuario carrega + `Load` |
| `Load(AssemblyName)` | Sempre retorna `null` ("Default Context nao pode sobrescrever a si mesmo") | Override do usuario, executado ANTES do fallback ao Default |
| Fallback ao Default | Nao existe (nao ha pai) | Sim, passo 3 da ordem de resolucao |
| Evento `Resolving` | Sim, e o ultimo recurso | Sim, e o ultimo recurso |
| Evento `AppDomain.AssemblyResolve` | Sim | Sim (global, nao por contexto) |
| `Unload()` | `InvalidOperationException` (nao coletavel) | Apenas se `isCollectible: true` |
| `System.Private.CoreLib` | Carregado aqui, uma vez | Nao pode ser carregado |
| Satelites | Resolvidos em PLATFORM_RESOURCE_ROOTS / APP_PATHS, depois `ResolveSatelliteAssembly` gerenciado | Resolvidos via `ResolveSatelliteAssembly` gerenciado (diretorio do assembly pai) |
| P/Invoke | Busca padrao do host (`NATIVE_DLL_SEARCH_DIRECTORIES`) | `LoadUnmanagedDll` / `ResolvingUnmanagedDll` antes da busca padrao |

### Inicializacao preguicosa do lado gerenciado do Default

Em `DefaultAssemblyBinder::BindUsingAssemblyName` (`defaultassemblybinder.cpp`) ha um detalhe revelador: o objeto gerenciado do ALC Default **nao e criado no startup**. Quando o bind nativo falha com `ERROR_FILE_NOT_FOUND`/`FUSION_E_APP_DOMAIN_LOCKED`/`FUSION_E_REF_DEF_MISMATCH`:

- Se `GetAssemblyLoadContext()` ainda e nulo e o pedido e de **assembly satelite** (`!IsNeutralCulture()`), o runtime chama `METHOD__ASSEMBLYLOADCONTEXT__INITIALIZE_DEFAULT_CONTEXT` para criar o ALC gerenciado, porque a logica extra de satelites vive em C# (`ResolveSatelliteAssembly`).
- Para assemblies nao-satelites, o comentario diz: qualquer logica adicional vem de um handler de evento, que ja teria inicializado o ALC gerenciado; se ele continua nulo, nao ha nada a executar.

Ou seja: o caminho feliz de uma aplicacao tipica nunca instancia o `AssemblyLoadContext` gerenciado do Default. E uma otimizacao de startup: o caminho rapido e 100% nativo.

### Como o Default e preenchido

Quem monta o Default e o **host** (hostfxr/hostpolicy), nao o runtime: ele passa propriedades ao `coreclr_initialize`. As conhecidas estao em `src/native/corehost/host_runtime_contract.h`:

```c
#define HOST_PROPERTY_TRUSTED_PLATFORM_ASSEMBLIES "TRUSTED_PLATFORM_ASSEMBLIES"
#define HOST_PROPERTY_PLATFORM_RESOURCE_ROOTS     "PLATFORM_RESOURCE_ROOTS"
#define HOST_PROPERTY_APP_PATHS                   "APP_PATHS"
#define HOST_PROPERTY_NATIVE_DLL_SEARCH_DIRECTORIES "NATIVE_DLL_SEARCH_DIRECTORIES"
```

(Detalhes nos arquivos 03 e 04.) O runtime so consome. `docs/design/features/host-runtime-information.md` documenta cada propriedade.

### Exemplo: sobrescrever uma dependencia do app num contexto customizado

```csharp
sealed class PluginContext : AssemblyLoadContext
{
    private readonly AssemblyDependencyResolver _resolver;

    public PluginContext(string pluginPath) : base(Path.GetFileName(pluginPath))
        => _resolver = new AssemblyDependencyResolver(pluginPath);

    protected override Assembly? Load(AssemblyName name)
    {
        // Passo 2: roda ANTES do fallback ao Default.
        string? path = _resolver.ResolveAssemblyToPath(name);
        return path is null ? null : LoadFromAssemblyPath(path);
    }
}
```

Se `Load` retornar `null`, o binder tenta o Default (passo 3), e so entao dispara `Resolving` (passo 5). Para *forcar* o uso da copia privada de uma biblioteca que tambem existe no Default, o plugin devolve a sua copia em `Load`.

### Limites do Default: quem ganha na TPA

`docs/design/features/native-hosting-load-context.md` documenta a limitacao direta: "Quando um componente carrega no ALC Default, os assemblies do framework vencem, entao ele nao pode sobrescreve-los com os seus proprios ou de versao mais alta." E tambem o contrario: "um ALC isolado por assembly dificulta compartilhar tipos e estado estatico". A solucao introduzida (novo `load_context` com `identifier`, mapeamento processo-global de identificador para ALC) cria **ALCs compartilhados e nomeados** sem serem o Default.

### Fluxo completo de um pedido de assembly (diagrama textual)

```
codigo em Plugin.dll (ALC "P") referencia Lib, Version=2.0.0.0
  |
  v
CustomAssemblyBinder("P")::BindUsingAssemblyName
  |-- 1. ApplicationContext de "P": ja ligado/falhou?  --sim--> resultado
  |        (BindAssemblyByNameWorker -> AssemblyBinderCommon::BindAssembly)
  |-- nao (FILE_NOT_FOUND | APP_DOMAIN_LOCKED | REF_DEF_MISMATCH)
  |-- 2. BindUsingHostAssemblyResolver(P)
  |        -> managed AssemblyLoadContext.Resolve -> P.Load(name)   --assembly--> ok
  |        -> (nulo) -> DefaultBinder::BindUsingAssemblyName(name)  --assembly--> ok (3)
  |        -> (satelite) -> P.ResolveSatelliteAssembly(name)        --assembly--> ok (4)
  |        -> P.Resolving event handlers                            --assembly--> ok (5)
  |-- 6. FileNotFoundException
```

Observacoes tiradas do codigo:

- `SetBinder(this)` so e chamado se o assembly devolvido ainda **nao tem binder** (`pCoreCLRFoundAssembly->GetBinder() == NULL`). Se o assembly veio do Default, ele **continua pertencendo ao Default**: nao e "reparentado" no contexto customizado.
- O comentario explicita que o passo 3 e pulado para satelites, que seguem para o passo 4.
- No `DefaultAssemblyBinder`, `FUSION_E_APP_DOMAIN_LOCKED` (versao incompativel) e traduzido para `FUSION_E_REF_DEF_MISMATCH`, de modo que, para o chamador, "versao mais nova pedida que a ja ligada" e "identidade nao casa" parecem o mesmo erro.

### Perguntas frequentes (respondidas pelo codigo)

| Pergunta | Resposta | Onde |
|---|---|---|
| O Default pode ser descarregado? | Nao: `Unload()` lanca `InvalidOperationException` se `!IsCollectible` | `AssemblyLoadContext.cs` ~linha 499 |
| Posso criar um ALC "coletavel" derivando `AssemblyLoadContext`? | Sim: `base(isCollectible: true)` ou `base(name, true)` | construtores nas linhas ~71-83 |
| O que `AssemblyLoadContext.All` enumera? | Todos os contextos vivos (via `WeakReference`), inclusive o Default e os anonimos | `AllContexts`, linha ~273 |
| Como pego o ALC de um assembly? | `AssemblyLoadContext.GetLoadContext(assembly)` | usado em `ResolveSatelliteAssembly` |
| Existe ALC em NativeAOT? | So o Default; `GetLoadContext` devolve `Default` sempre | `AssemblyLoadContext.NativeAot.cs` |
| O que acontece com um `Load` que devolve assembly de outro nome? | `InvalidOperationException` (`ValidateAssemblyNameWithSimpleName`) | `AssemblyLoadContext.cs` |
| `Load` e chamado com lock? | Nao; o binder apenas invoca o callback. Implementacoes devem ser thread-safe | `ResolveUsingLoad` |

### Contexto "compartilhado nomeado" via hosting nativo

O doc `native-hosting-load-context.md` estende o modelo: parametro `load_context` com `NULL` (Default), `ISOLATED_CONTEXT` ou um ponteiro para `struct load_context { size_t size; const char_t *identifier; }`. O runtime mantem um mapa `identifier -> ALC` apenas para contextos criados por hosting nativo; comparacao ordinal; o identificador e global do processo, portanto deve ser prefixado por empresa/aplicacao. Na resolucao, cada componente adiciona um `AssemblyDependencyResolver` ao `Load` do contexto, consultados na ordem de carga dos componentes (o primeiro vence).

## Por que

- **Default = "o mundo do app"**: um unico contexto onde o grafo estatico do aplicativo (descrito pelo `.deps.json`) resolve de forma barata e previsivel; e a "plataforma" que todos os plugins compartilham.
- **Impossibilidade de sobrescrever o Default a partir de dentro** (Load sempre `null`) garante um ponto fixo: o que a plataforma entrega e o que vale. A flexibilidade esta nos contextos filhos.
- **Custom antes do fallback** (passo 2 antes do 3) e a decisao-chave do design: "Este approach enables a LoadContext to override assemblies that have been loaded in TPA context by loading a different (or even the same!) version" (comentario em `customassemblybinder.cpp`).
- **Binder nativo separado por contexto** permite que o contexto Default tenha uma tabela pre-populada (TPA) e os custom tenham so cache dinamico.

## Trade-offs

- Dois "sabores" de contexto significam duas implementacoes de binder para manter (`DefaultAssemblyBinder`, `CustomAssemblyBinder`) e testes cruzados.
- O fallback implicito ao Default torna o **isolamento parcial** por padrao: a maioria dos plugins "vaza" para tipos do host se `Load` nao for sobrescrito.
- `Resolving` chega por ultimo; o usuario que quer *prioridade* precisa de `Load`, o que exige herdar a classe (nao basta se inscrever num evento).
- O startup rapido depende da inicializacao preguicosa; a semantica "Load do Default sempre null" e exatamente o que permite pular o gerenciado.
- ALCs anonimos (`LoadFile`, `Load(byte[])`) criam contextos que o usuario nao nomeia e que nao compartilham nada: facil gerar duplicidade de tipos sem querer.

## Como aplicar na sua linguagem

1. **Tenha um escopo "raiz" distinto, populado pelo launcher/host** e imutavel pelo usuario, e escopos derivados criados pelo usuario. Modele isso como dois *tipos* (ou um tipo com flag `is_root`), nao como convencao.
2. **Mantenha o caminho rapido sem codigo do usuario**: resolva primeiro numa tabela pre-computada do escopo raiz (nome -> arquivo), e so invoque hooks do usuario em caso de miss. O .NET so chama o gerenciado depois de `FILE_NOT_FOUND`/`REF_DEF_MISMATCH`.
3. **Dê ao escopo derivado precedencia sobre o pai** por padrao *opt-in* (um hook `load` que roda antes). Se sua linguagem quer "isolamento por padrao", inverta: pai so se o modulo estiver numa lista de "compartilhados" declarada pelo plugin.
4. **Permita escopos nomeados e compartilhados** (como o `identifier` do `load_context`): dois plugins relacionados querem o mesmo escopo sem tocar o raiz.
5. **Proiba carregar o modulo base fora do raiz.** Documente a lista de "modulos de plataforma" (equivalente a CoreLib) e faca a violacao um erro claro, nao um bug de identidade de tipo.
6. **Nao crie objetos de reflexao/gerenciamento do raiz ate que sejam necessarios** (otimizacao de startup observada no Default).
7. **Exponha `current_scope()`** e um `with scope(x):` (analogo a `EnterContextualReflection`) para APIs de carga por string.

## Referencias

- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` (Default, Individual)
- `src/coreclr/binder/defaultassemblybinder.cpp`, `src/coreclr/binder/customassemblybinder.cpp`
- `src/coreclr/binder/applicationcontext.cpp` (`SetupBindingPaths`)
- `src/native/corehost/host_runtime_contract.h`
- `docs/design/features/assemblyloadcontext.md`
- `docs/design/features/native-hosting-load-context.md`
- `docs/design/features/host-runtime-information.md`
- ECMA-335: nao especifica contextos de carga; apenas resolucao de referencias de assembly (Partition II, 6.3 "Referencing assemblies").
