# AssemblyLoadContext: isolamento e grafos de carga

> Objetivo: entender o que e um "contexto de carga" no .NET, como ele particiona o mundo de modulos carregados e o que isso ensina para o desenho de uma nova linguagem.

## Mecanismo no .NET

### O que e um contexto de carga

Segundo `docs/design/features/assemblyloadcontext.md`, um **LoadContext** e "um container para assemblies, seu codigo e dados (por exemplo, statics)". Toda vez que um assembly e carregado, ele e carregado *dentro* de um contexto, independentemente de a carga ser explicita (`Assembly.Load`), implicita (resolver referencias estaticas do manifesto) ou dinamica (codigo emitido em tempo de execucao).

A classe publica e `System.Runtime.Loader.AssemblyLoadContext`, implementada em duas camadas:

| Camada | Arquivo | Papel |
|---|---|---|
| Gerenciada, comum | `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` | API publica, eventos, estado do ciclo de vida |
| Gerenciada, CoreCLR | `src/coreclr/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.CoreCLR.cs` | QCalls (`AssemblyNative_*`) para o runtime nativo |
| Gerenciada, NativeAOT | `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs` | Stub: so o contexto Default existe |
| Nativa (binder) | `src/coreclr/binder/` (`defaultassemblybinder.cpp`, `customassemblybinder.cpp`, `assemblybindercommon.cpp`) | Algoritmo de bind por nome |
| Nativa (VM) | `src/coreclr/vm/assemblynative.cpp`, `loaderallocator.cpp` | Posse de memoria e tipos |

### Cada ALC gerenciado tem um "binder" nativo

O construtor privado em `AssemblyLoadContext.cs` (linha ~83) chama `InitializeAssemblyLoadContext(thisHandlePtr, representsTPALoadContext, isCollectible)`, que e o QCall `AssemblyNative_InitializeAssemblyLoadContext`. O retorno (`_nativeAssemblyLoadContext`) e um ponteiro para um `AssemblyBinder` nativo:

- `DefaultAssemblyBinder` (`src/coreclr/binder/defaultassemblybinder.cpp`) para o contexto Default.
- `CustomAssemblyBinder` (`src/coreclr/binder/customassemblybinder.cpp`) para qualquer outro.

O ALC gerenciado guarda um `GCHandle` para si mesmo e entrega ao nativo: se o ALC e coletavel o handle e `WeakTrackResurrection`, caso contrario `Normal`. Isso e o ponto de ancoragem do ciclo de vida (ver arquivo 08).

Tambem ha um registro global fraco de todos os contextos (`AllContexts`, `s_nextId`, `AssemblyLoadContext.All`), usado para enumeracao e para disparar `Unloading` no encerramento do processo (`OnProcessExit`).

### O grafo de carga

O "grafo de carga" nao e uma estrutura explicita; emerge de duas regras:

1. **Regra do solicitante**: se o assembly `A1` dispara a carga de `C1`, a tentativa e feita no contexto de `A1` (o *RequestingAssembly*). Assemblies dinamicos (Reflection.Emit) herdam o contexto do *Creator Assembly*. (`assemblyloadcontext.md`, secao "Basics".)
2. **Cache de bind por contexto**: cada binder tem um `ApplicationContext` (`src/coreclr/binder/applicationcontext.cpp`) com a tabela de assemblies ja ligados *e* um cache de falhas (`FailureCache`). Entao "ja carregado ou ja falhou" e uma propriedade do contexto, nao do processo.

Consequencia direta: **dois assemblies com o mesmo nome simples nao podem coexistir em um contexto, mas podem coexistir em contextos diferentes**. Os tipos sao distintos (identidade de tipo = identidade de assembly + contexto), e os `static`s sao duplicados por contexto.

### Ordem de resolucao em um contexto customizado

O comentario normativo esta em `CustomAssemblyBinder::BindUsingAssemblyName` (`src/coreclr/binder/customassemblybinder.cpp`):

```
1) Procurar o assembly no proprio LoadContext. Se achar, usar.
2) Invocar o override Load() do LoadContext. Se achar, usar.
3) Procurar no DefaultBinder (exceto para satelites). Se achar, usar.
4) Invocar ResolveSatelliteAssembly do LoadContext (para satelites).
5) Disparar o evento Resolving do LoadContext.
6) Lancar excecao.
```

Em codigo gerenciado, os passos 2, 4 e 5 chegam por callbacks `[UnmanagedCallersOnly]` em `AssemblyLoadContext.cs`: `Resolve` (-> `ResolveUsingLoad` -> `Load`), `ResolveSatelliteAssembly` e `ResolveUsingEvent`. Note `ValidateAssemblyNameWithSimpleName`: o assembly devolvido por `Load`/`Resolving` **precisa ser um `RuntimeAssembly` e ter o mesmo nome simples** que o solicitado (comparacao case-insensitive); caso contrario lanca `InvalidOperationException`.

### APIs de carga e a qual contexto elas pertencem

De `assemblyloadcontext.md`:

| API | Contexto onde carrega |
|---|---|
| `Assembly.Load(name)` | Contexto do assembly que dispara a carga |
| `Assembly.LoadFrom` | Default |
| `Assembly.LoadFile` | Novo contexto anonimo (`IndividualAssemblyLoadContext`) |
| `Assembly.Load(byte[])` | Novo contexto anonimo |
| `alc.LoadFromAssemblyPath(path)` | Esse `alc` (carga "forcada" por caminho) |
| `alc.LoadFromStream(stream[, pdb])` | Esse `alc` |
| `alc.LoadFromAssemblyName(name)` | Inicia um *bind* em `alc`; pode terminar resolvido no Default |

`LoadFromAssemblyName` **nao** forca o assembly para dentro do contexto: ele inicia o algoritmo completo de bind. Somente `LoadFromAssemblyPath`/`LoadFromStream` garantem que os bytes pertencem ao contexto. `LoadFromAssemblyPath` exige caminho totalmente qualificado (`PathInternal.IsPartiallyQualified` -> `ArgumentException`) e faz `lock (_unloadLock); VerifyIsAlive();` antes de chamar `InternalLoadFromPath` -> QCall `AssemblyNative_LoadFromPath`.

### Restricoes documentadas

- `System.Private.CoreLib` e carregado uma unica vez, no Default; o binder custom o rejeita (`IsCoreLib()` -> `ERROR_FILE_NOT_FOUND` em `CustomAssemblyBinder::BindUsingPEImage`).
- Imagens ReadyToRun carregadas do mesmo caminho em varios contextos so usam codigo pre-compilado na primeira; as demais sao JITadas (mapeamento do SO compartilhado).
- Validacao de arquitetura (`IsValidArchitecture`) acontece antes do bind.

### Estado contextual (reflexao)

Como o "contexto inferido" pode ser ambiguo (um plugin chama uma lib que faz `Type.GetType("X, Y")`), o runtime oferece `AssemblyLoadContext.CurrentContextualReflectionContext` (um `AsyncLocal`) e `EnterContextualReflection()` que devolve um `ContextualReflectionScope : IDisposable` (`AssemblyLoadContext.cs`, ~linhas 565-640). Design completo em `docs/design/features/AssemblyLoadContext.ContextualReflection.md`. Regra de inferencia: metodo estatico -> ALC do assembly do chamador; metodo de instancia -> ALC do assembly da instancia.

### Exemplo minimo: dois contextos, duas identidades

```csharp
using System.Reflection;
using System.Runtime.Loader;

sealed class IsolatedContext : AssemblyLoadContext
{
    public IsolatedContext(string name) : base(name, isCollectible: false) { }

    // Passo 2 da ordem de resolucao: antes do fallback ao Default.
    protected override Assembly? Load(AssemblyName name) => null; // null => cai no Default
}

var a = new IsolatedContext("A");
var b = new IsolatedContext("B");

Assembly asmA = a.LoadFromAssemblyPath(@"C:\libs\Lib.dll");
Assembly asmB = b.LoadFromAssemblyPath(@"C:\libs\Lib.dll");

// Mesmos bytes, mesmo caminho, contextos diferentes:
bool sameAssembly = ReferenceEquals(asmA, asmB);              // false
bool sameType = asmA.GetType("Lib.Foo") == asmB.GetType("Lib.Foo"); // false
// Statics de Lib.Foo existem em duas copias independentes.
// E (nota do doc) o codigo R2R so e aproveitado em um dos dois; o outro e JITado.
```

Enumeracao e inspecao (todas em `AssemblyLoadContext.cs`):

```csharp
foreach (AssemblyLoadContext alc in AssemblyLoadContext.All)
    Console.WriteLine($"{alc} collectible={alc.IsCollectible}, assemblies={alc.Assemblies.Count()}");

AssemblyLoadContext? owner = AssemblyLoadContext.GetLoadContext(typeof(Foo).Assembly);
```

### Maquina de estados do contexto

O campo `_state` (`InternalState.Alive` / `Unloading`) e checado por `VerifyIsAlive()` em todo caminho de carga (`LoadFromAssemblyPath`, `LoadFromStream`, ...), sempre sob `lock (_unloadLock)`. Ou seja, o contexto e uma maquina de estados pequena: `Alive -> Unloading`, com a unica transicao ocorrendo uma vez em `InitiateUnload()`. Isso importa para o design: **carga concorrente com descarregamento precisa de um lock e de um estado terminal explicito**, nao de "melhor esforco".

### O que o binder nativo decide, em ordem

Dentro de `AssemblyBinderCommon::BindByName` (`assemblybindercommon.cpp`, ~linha 393):

1. Consulta o `FailureCache` pelo *display name com versao*; falhas anteriores nao-transitorias sao repetidas sem tocar o disco.
2. Rejeita referencias com arquitetura invalida (`FUSION_E_INVALID_NAME`).
3. `BindLocked`: procura no contexto de execucao (`FindInExecutionContext`); se houver candidato, aplica `IsCompatibleAssemblyVersion` (ver arquivo 05); se nao houver, `BindByTpaList` (ver arquivos 03 e 04).
4. Qualquer falha nao-transitoria e registrada no cache de falhas.

Somente quando o resultado e `ERROR_FILE_NOT_FOUND`, `FUSION_E_APP_DOMAIN_LOCKED` ou `FUSION_E_REF_DEF_MISMATCH` o binder custom/Default chama o gerenciado (`BindUsingHostAssemblyResolver`). Ou seja, **o codigo do usuario so e consultado como excecao**, e a parte "quente" (assemblies ja ligados) permanece nativa e rapida.

## Por que

- **Multiplas versoes no mesmo processo** (plugins), **isolamento de statics**, **sobrescrever assemblies do app** e "expor o LoadContext como conceito de primeira classe e nao magica" sao os quatro motivadores listados literalmente em `assemblyloadcontext.md`.
- Vem do .NET Framework (load contexts Default/LoadFrom/Neither, ver post de Suzanne Cook citado no doc), onde era implicito e confuso. A decisao foi *nomear e expor* o conceito.
- A substituicao dos AppDomains: AppDomain unia isolamento de codigo, de dados, de seguranca e unload. O ALC fica so com resolucao + posse de memoria (ver `unloadability.md`, secoes sobre Roslyn/ASP.NET/LINQPad, onde se conclui que AppDomains davam "falsa sensacao de seguranca").
- A cadeia "proprio contexto -> Load -> Default -> Resolving" faz o Default funcionar como **fallback compartilhado**: tipos de framework (`System.Object`) tem identidade unica por processo, essencial para troca de objetos entre host e plugin.

## Trade-offs

- **Identidade de tipo fragmentada**: o mesmo `Foo` em dois ALCs sao tipos diferentes; cast entre eles falha com `InvalidCastException` mesmo com bytes identicos. Pior diagnostico: a mensagem diz "cannot cast Foo to Foo".
- **Memoria e tempo de JIT duplicados**: statics, tabelas de tipo e codigo JITado por contexto; R2R so uma vez por caminho.
- **Fallback implicito ao Default** vence silenciosamente: se o Default ja tem `Newtonsoft.Json` 13 e o plugin quer 12, o passo 3 so e alcancado se `Load` retornar `null` -- o plugin precisa interceptar em `Load` para sobrescrever.
- **`LoadFromAssemblyName` enganoso**: quem espera "carregue neste contexto" recebe as vezes o assembly do Default.
- **Ambiguidade de contexto em reflexao** exigiu um mecanismo adicional (`AsyncLocal` contextual).
- **Custo de API**: ordem de busca em 6 passos e dificil de explicar; o runtime acrescentou tracing (`TraceResolvingHandlerInvoked`, `BinderTracing`) para depuracao.

## Como aplicar na sua linguagem

1. **Torne "namespace de modulos" um objeto de primeira classe** (`Loader`/`ModuleScope`) desde a v1, com identidade de modulo = (nome, escopo). Evite um unico dicionario global `name -> module`.
2. **Defina a ordem de resolucao em tabela publica e testavel.** Sugestao copiada do .NET, simplificada:
   ```
   resolve(scope, ref):
     1. scope.cache[ref.name]            (inclui falhas, para determinismo)
     2. scope.hook_load(ref)             (override do usuario, ANTES do pai)
     3. scope.parent.resolve(ref)        (pai/compartilhado explicito)
     4. scope.hook_resolving(ref)        (ultimo recurso, so notificacao)
     5. erro com trilha de tentativas
   ```
   Torne o "pai" um parametro do escopo, nao um global implicito: o .NET fixa Default como unico pai; voce pode permitir um DAG de escopos.
3. **Exija que o resultado do hook tenha o mesmo nome pedido** (como `ValidateAssemblyNameWithSimpleName`); isso evita bugs de "troquei o modulo por outro".
4. **Separe `load_by_path/bytes` (forca posse) de `load_by_name` (negocia)** e documente isso no nome das funcoes.
5. **Faca a identidade de tipo incluir o escopo** e dê um erro de cast que cite os dois escopos, mitigando o "Foo nao e Foo".
6. **Ofereca um escopo "reflexao contextual"** (variavel dinamica/async-local) para APIs de carga por string; sem isso frameworks de serializacao quebram em plugins.
7. **Decida cedo o que e sempre compartilhado** (o equivalente a CoreLib): runtime base nao pode ser carregado em escopo custom.
8. **Instrumente a resolucao**: um log estruturado de "quem tentou, em que ordem, quem ganhou" (analogo a `BinderTracing`) custa pouco e economiza semanas de suporte.

## Referencias

- `docs/design/features/assemblyloadcontext.md`
- `docs/design/features/AssemblyLoadContext.ContextualReflection.md`
- `docs/design/features/unloadability.md`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.CoreCLR.cs`
- `src/coreclr/nativeaot/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.NativeAot.cs`
- `src/coreclr/binder/customassemblybinder.cpp`, `defaultassemblybinder.cpp`, `assemblybindercommon.cpp`, `applicationcontext.cpp`
- Especificacao: ECMA-335 Partition II secao 6 (Assemblies) e Partition I secao 9.4 do tipo (identidade de tipo); o ALC em si **nao** e definido pela ECMA-335 -- e comportamento de implementacao.
