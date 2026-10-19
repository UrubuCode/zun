# Module initializers e startup hooks

## Mecanismo no .NET

Há três mecanismos relacionados, em níveis diferentes, para "executar código antes do programa começar":

| Mecanismo | Nível | Quem aciona | Onde está definido |
|---|---|---|---|
| **Type initializer (`.cctor`)** | por tipo | runtime, na primeira utilização | ECMA-335 II.10.5.3 |
| **Module initializer** | por módulo (assembly) | runtime, antes do primeiro acesso a membro estático ou método do módulo | `Ecma-335-Augments.md` ("Module Initializer") |
| **Startup hook** | por processo | host/ambiente (`DOTNET_STARTUP_HOOKS`) | `docs/design/features/host-startup-hook.md` |

### Module initializer (especificação)

De `docs/design/specs/Ecma-335-Augments.md`, seção "Module Initializer":

> All modules may have a module initializer. A module initializer is defined as the type initializer (§ II.10.5.3) of the `<Module>` type (§ II.10.8).

`<Module>` é o pseudo-tipo `TypeDef` (primeira linha de `TypeDef`) que contém métodos e campos **globais** do módulo. Seu `.cctor` é o module initializer. Não há limitação sobre o que ele pode fazer: "Module initializers are permitted to run and call both managed and unmanaged code."

**Garantias** (além das de qualquer type initializer):

1. executa *"at, or sometime before, first access to any static field or first invocation of any method defined in the module"*;
2. executa **exatamente uma vez** por módulo, salvo chamada explícita do usuário;
3. *"No method other than those called directly or indirectly from the module initializer will be able to access the types, methods, or data in a module before its initializer completes execution."*

Ou seja, a spec **não** diz "antes de `Main`" nem especifica ordem entre módulos — apenas "antes do primeiro acesso a este módulo" (execução *preguiçosa*, a menos que a implementação decida ser eager).

### Como a linguagem expõe: `[ModuleInitializer]`

`System.Runtime.CompilerServices.ModuleInitializerAttribute` (`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ModuleInitializerAttribute.cs`): *"Used to indicate to the compiler that a method should be called in its containing module's initializer."* O compilador, ao ver métodos marcados, **gera** um `.cctor` em `<Module>` que chama cada um. Requisitos (do XML doc): `static`, método comum (não accessor/ctor/local function), sem parâmetros, retorna `void`, não genérico nem em tipo genérico, acessibilidade efetiva `internal` ou `public`. Ou seja: o recurso do runtime é só o `.cctor` de `<Module>`; o atributo é *açúcar de compilador*.

### Implementação no CoreCLR

- Ao carregar o módulo/entrypoint, o runtime dispara o `.cctor` de `<Module>` quando necessário. Para forçar explicitamente: `RuntimeHelpers.RunModuleConstructor(ModuleHandle)` (`src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs`), que chama o QCall `ReflectionInvocation_RunModuleConstructor`. Comentário no código: *"After this call returns, the module constructor is guaranteed to have at least been started by some thread. In the absence of module constructor deadlock conditions, the call is further guaranteed to have completed."* e propaga exceção se o initializer lançou.
- Deadlocks são possíveis como em qualquer `.cctor` (locks de inicialização entre threads/módulos).

### Implementação no Native AOT

Como não há carregamento preguiçoso de módulos, o ILC decide ordem **em tempo de compilação**:

1. `ModuleInitializerListNode` (`src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/DependencyAnalysis/ModuleInitializerListNode.cs`) coleta os módulos cujo `<Module>` tem `.cctor` compilado (`mdType.IsModuleType && method.IsStaticConstructor`).
2. Faz **ordenação topológica por referências de assembly** (os módulos que não referenciam outros primeiro): *"This is an approximation that tries to deal with module initializers that might have dependencies on each other. The spec doesn't guarantee any ordering so this is best effort."* Em ciclos, escolhe uma "vítima". O comentário explica o contra-intuitivo: o initializer do módulo de entrada tende a rodar por último, mas se ele chamar outro módulo com `.cctor`, o CoreCLR rodaria o dependente primeiro — a ordenação imita isso.
3. Emite um array simbólico `__module_initializers` com ponteiros (relativos de 32 bits se o alvo suporta) para cada `.cctor` de módulo. Nota no código: no modo *multi-object-module* essa lista plana é insuficiente para ordenação.
4. No startup, o método sintetizado `StartupCodeMain` (`StartupCodeMainMethod.cs`) chama `StartupCodeHelpers.RunModuleInitializers()` (`src/coreclr/nativeaot/Common/src/Internal/Runtime/CompilerHelpers/StartupCodeHelpers.cs`), que itera `s_modules` e, para cada, lê a seção `ReadyToRunSectionType.ModuleInitializerList` e invoca cada ponteiro `delegate*<void>`.

A ordem de `StartupCodeMain`: initializers de biblioteca → `InitializeCommandLineArgs[W]` → `InitializeEntryAssembly` → `InitializeApartmentState` → **`RunModuleInitializers`** → `Main`. Em executáveis, os module initializers rodam *antes* de `Main` (eager); em bibliotecas nativas existe `NativeLibraryStartupMethod`. Em `Program.cs` do ILC, a flag `generateLibraryAndModuleInitializers` é falsa para certos modos (ex.: parte do split exe).

### Startup hooks (CoreCLR)

`docs/design/features/host-startup-hook.md`: variável de ambiente `DOTNET_STARTUP_HOOKS` com lista (separada por `;` no Windows, `:` no Unix) de caminhos absolutos ou nomes simples de assembly. Cada assembly deve conter um tipo `StartupHook` (sem namespace, preferencialmente `internal`) com `public static void Initialize()`. Eles rodam **na mesma thread de `Main`**, em ordem, **antes** de `Main` e antes do assembly principal ser carregado.

Fluxo real no código:

1. O host (`hostpolicy`) inicia o CoreCLR e passa a propriedade `STARTUP_HOOKS` (acessível por `AppContext.GetData("STARTUP_HOOKS")`).
2. `Assembly::ExecuteMainMethod` (`src/coreclr/vm/assembly.cpp`) chama `RunManagedStartup()` imediatamente **antes** de `RunMain(...)`. `RunManagedStartup` invoca (via `UnmanagedCallersOnlyCaller`) `StartupHookProvider.ManagedStartup` (`src/coreclr/System.Private.CoreLib/src/System/StartupHookProvider.CoreCLR.cs`), passando também os hooks de diagnóstico.
3. `StartupHookProvider.ProcessStartupHooks` (`src/libraries/System.Private.CoreLib/src/System/StartupHookProvider.cs`) junta hooks de diagnóstico + variável de ambiente, valida sintaxe (`ParseStartupHook`) e chama cada um (`CallStartupHook`): carrega o assembly (por caminho, ou por nome via `AssemblyLoadContext.Default`), busca o tipo `StartupHook` (`StartupHookTypeName = "StartupHook"`), valida a assinatura do método e invoca.
4. Depois, `RunMain`.

**Feature switch**: `[FeatureSwitchDefinition("System.StartupHookProvider.IsSupported")] IsSupported` (default `true`); permite que o *trimmer*/compilador AOT remova o código. Existe aviso de trim porque carregar assemblies dinamicamente é incompatível com trimming (`[RequiresUnreferencedCode]` em `CallStartupHook`).

**Erros** (doc): sintaxe inválida ou caminho parcialmente qualificado → `ArgumentException` (antes de executar qualquer hook); assembly ausente → `FileNotFoundException`; inválido → `BadImageFormatException`; tipo ausente → `TypeLoadException`; `Initialize` ausente → `MissingMethodException`; assinatura incorreta → `ArgumentException`; exceção não tratada → mesma que em `Main`.

**Caveats do doc**: sem solução embutida para ordem entre hooks; sem resolução de dependências além da TPA (lista de assemblies confiáveis do app); sem resolução de conflitos entre hooks. Uso típico: injetar telemetria, ajustar `AssemblyLoadContext` (exemplo: `AssemblyLoadContext.Default.Resolving += ...` para um local compartilhado).

### Startup hooks em Native AOT

`src/coreclr/nativeaot/System.Private.CoreLib/src/System/StartupHookProvider.NativeAot.cs`:

```csharp
internal static partial class StartupHookProvider
{
    [ModuleInitializer]
    internal static void Initialize()
    {
        if (IsSupported)
            ProcessStartupHooks(Environment.GetEnvironmentVariable("DOTNET_STARTUP_HOOKS"));
    }
}
```

Isto é revelador: em AOT o próprio mecanismo de startup hook é implementado **usando um module initializer** no `System.Private.CoreLib`, já que não há host que chame `ManagedStartup`. Como o NativeAOT não carrega assemblies arbitrários (`InternalLoadFromPath` lança `PlatformNotSupportedException`, arquivo 07), a utilidade prática dos hooks nesse modelo é limitada (não verifiquei a política padrão do feature switch no build AOT).

## Por quê

- **Module initializer**: dar a bibliotecas um ponto de inicialização único sem exigir que o usuário chame `Init()`, sem a restrição de "tipos" (`static` ctor por tipo) e sem custo de verificar a cada acesso. Permite registrar serviços, ajustar `AppContext`, instalar *resolvers*, etc.
- **Execução preguiçosa (spec)**: evita pagar custo de módulos nunca usados, preserva startup.
- **Startup hook**: injeção de comportamento sem modificar o app (telemetria, políticas de carga), "separate from the entry point, so that user code doesn't need to be modified".
- **Ordem em AOT**: sem carga dinâmica, a resolução de ordem é feita estaticamente com boa aproximação.

## Trade-offs

| Escolha | Vantagem | Custo |
|---|---|---|
| Spec só garante "antes do primeiro acesso" | Implementações podem escolher lazy ou eager | Autor não pode contar com ordem entre módulos; testes comportamentais divergem entre CoreCLR e AOT |
| Reuso de `.cctor` de `<Module>` | Nenhum conceito novo de runtime | Herdam problemas de deadlock de type initializers e exceções que "envenenam" o módulo |
| Módulo com `.cctor` arrasta código | Fácil de usar | Impede *trimming*/lazy: o initializer mantém tudo que referencia (ex.: em AOT, entra no grafo de dependências) |
| Startup hook por variável de ambiente | Sem alterar binário | Herdada por processos filhos; ordem frágil; carrega código arbitrário — risco de segurança |
| Hook em tempo de `Main` na mesma thread | Previsível | Atrasa startup; exceção derruba o processo |
| Em AOT, `ModuleInitializer` implementa hooks | Reuso | Dependente de feature switch; não carrega código novo |

## Como aplicar na sua linguagem

1. **Defina "inicializador de módulo" como conceito de linguagem** mapeado em um símbolo conhecido (o `.cctor` de `<Module>`). Documente: exatamente uma vez, antes do primeiro uso, thread-safe, e o que acontece em caso de exceção.
2. **Decida eager × lazy** e especifique. Se seu runtime tiver AOT e JIT, escreva a spec de modo a permitir ambos (como o .NET) mas ofereça API de "forçar" (`RunModuleConstructor`).
3. **Documente o que NÃO é garantido**: ordem entre módulos. No AOT, calcule uma ordem determinística (topológica pelas referências) e emita uma lista de ponteiros numa seção dedicada (`ModuleInitializerList`).
4. **O compilador cria o initializer a partir de um atributo/anotação** (`@module_init`), validando assinatura (estático, sem parâmetros, `void`, não genérico, visibilidade). A runtime só vê o `.cctor`.
5. **Separe inicialização de runtime da de usuário**: no AOT, o startup sintético chama, nesta ordem, inicializadores de biblioteca, args, assembly de entrada, apartment, module initializers, `main`.
6. **Evite inicialização cara** em initializers: eles forçam código e dados para o grafo (e para o *working set*).
7. **Startup hooks**: se oferecer extensão por ambiente, torne-a um *feature switch* removível por trimming, restrinja a nomes/caminhos validados, execute na thread principal antes do `main`, e documente os modos de falha como o .NET faz.
8. **Cuidado com deadlocks**: se initializer de A chama B e B chama A em threads diferentes, é deadlock clássico — detecte ou documente.
9. **Segurança**: variável de ambiente que carrega código é vetor de ataque; permita desabilitar.

## Referências

- `docs/design/specs/Ecma-335-Augments.md` — "Module Initializer" e "Module Initialization Guarantees" (linhas ~370–384).
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/ModuleInitializerAttribute.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` — `RunModuleConstructor`.
- `src/coreclr/vm/assembly.cpp` — `RunManagedStartup`, `Assembly::ExecuteMainMethod`.
- `src/coreclr/System.Private.CoreLib/src/System/StartupHookProvider.CoreCLR.cs`, `src/libraries/System.Private.CoreLib/src/System/StartupHookProvider.cs`, `src/coreclr/nativeaot/System.Private.CoreLib/src/System/StartupHookProvider.NativeAot.cs`.
- `docs/design/features/host-startup-hook.md`.
- `src/coreclr/tools/aot/ILCompiler.Compiler/Compiler/DependencyAnalysis/ModuleInitializerListNode.cs`, `.../IL/Stubs/StartupCode/StartupCodeMainMethod.cs`, `src/coreclr/nativeaot/Common/src/Internal/Runtime/CompilerHelpers/StartupCodeHelpers.cs`.
- ECMA-335 II.10.5.3 (type initializers), II.10.8 (tipo `<Module>` / membros globais).

## Apêndice A — Exemplos de código

Module initializer em C# (açúcar sobre o `.cctor` de `<Module>`):

```csharp
using System.Runtime.CompilerServices;

static class Bootstrap
{
    [ModuleInitializer]            // exige: static, void, sem parâmetros, não genérico, internal/public
    internal static void Init()
    {
        // executa uma vez, antes do primeiro acesso ao módulo
        AppContext.SetSwitch("MyLib.Feature", true);
    }
}
```

Startup hook (assembly separado, apontado por `DOTNET_STARTUP_HOOKS`):

```csharp
// sem namespace, tipo preferencialmente internal
internal class StartupHook
{
    public static void Initialize()      // public static void, sem parâmetros
    {
        System.Console.WriteLine("hook executado antes de Main");
    }
}
```

Execução:

```
# Windows
set DOTNET_STARTUP_HOOKS=D:\hooks\MyHook.dll
# Unix
export DOTNET_STARTUP_HOOKS=/hooks/MyHook.dll
dotnet MyApp.dll
```

(Sintaxe e requisitos conforme `docs/design/features/host-startup-hook.md`.)

## Apêndice B — Linha do tempo de um processo CoreCLR (trecho relevante)

1. O host (`hostfxr`/`hostpolicy`) resolve o framework e a TPA e inicia o runtime com as propriedades (`STARTUP_HOOKS` incluso).
2. O runtime carrega o assembly principal e localiza o entry point (`GetEntryPoint()` em `assembly.cpp`).
3. `Assembly::ExecuteMainMethod`: ajusta estado de apartment (COM), `RunMainPre()`, define o *root assembly*, registra o módulo R2R se for o caso e inicializa a thread gerenciada principal.
4. `RunManagedStartup()` → `StartupHookProvider.ManagedStartup(...)` → hooks.
5. `RunMain(...)` → o `.cctor` de `<Module>` do assembly principal dispara quando o código do módulo é acessado pela primeira vez (spec: "at, or sometime before, first access"), e então `Main` executa.

## Apêndice C — Linha do tempo de um executável Native AOT

1. `main()` nativo do bootstrapper → `StartupCodeMain` (IL sintetizado pelo ILC).
2. Inicializadores de biblioteca explicitamente ordenados → args → assembly de entrada → apartment.
3. `RunModuleInitializers`: para cada módulo na ordem topológica calculada em build, chama o `.cctor` do `<Module>`; entre eles está o `StartupHookProvider.Initialize` do CoreLib, que processa `DOTNET_STARTUP_HOOKS` se `IsSupported`.
4. `Main` do usuário.

## Apêndice D — Armadilhas

1. **Dependência de ordem entre módulos**: a spec não garante; o ILC só aproxima (topologia por AssemblyRef, com "vítima" em ciclos).
2. **Efeito em trimming**: um module initializer é raiz implícita do módulo — tudo que ele toca é mantido.
3. **Exceção no initializer**: como em `.cctor`, o módulo pode ficar inutilizável; `RunModuleConstructor` repropaga.
4. **Threads**: o initializer pode rodar na primeira thread que toca o módulo; evite bloquear em outras threads que também precisem do módulo.
5. **Hooks e segurança**: variável de ambiente é herdada por processos filhos (o doc recomenda limpar se necessário).
