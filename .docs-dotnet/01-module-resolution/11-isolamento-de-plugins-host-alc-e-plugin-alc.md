# Isolamento de plugins (host ALC + plugin ALC)

> Objetivo: juntar as pecas dos arquivos anteriores no padrao mais usado em producao: um aplicativo host no contexto Default e cada plugin em seu proprio `AssemblyLoadContext`, com um `AssemblyDependencyResolver` apontando para o `.deps.json` do plugin, e uma "fronteira de contrato" de tipos compartilhados.

## Mecanismo no .NET

### As tres pecas que o runtime fornece

1. **Um contexto por plugin** (`class PluginLoadContext : AssemblyLoadContext`), com resolucao propria em `Load` e `LoadUnmanagedDll` (arquivos 01, 02, 09).
2. **`AssemblyDependencyResolver`** (`src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`), que reaproveita o codigo de `hostpolicy` (via `corehost_resolve_component_dependencies`, `src/native/corehost/hostpolicy/hostpolicy.cpp` ~linha 917) para ler o `<plugin>.deps.json` e responder: `ResolveAssemblyToPath(AssemblyName)` e `ResolveUnmanagedDllToPath(string)`.
3. **Regra de fallback ao Default** no binder custom: se `Load` devolve `null`, o assembly e procurado no Default (`CustomAssemblyBinder::BindUsingAssemblyName`, `src/coreclr/binder/customassemblybinder.cpp`). Assim tipos do framework e do contrato (que o host ja carregou) sao compartilhados automaticamente.

### O `AssemblyDependencyResolver` em detalhe

Construtor (`AssemblyDependencyResolver(string componentAssemblyPath)`):

- Instala um *error writer* thread-local no hostpolicy (`corehost_set_error_writer`) para capturar mensagens como texto e as inclui na `InvalidOperationException` em caso de falha (`SR.AssemblyDependencyResolver_FailedToResolveDependencies`).
- Chama `corehost_resolve_component_dependencies(path, callback)`; o callback recebe tres strings: lista de assemblies (caminhos), diretorios nativos e diretorios de recursos.
- Falhas de carga do hostpolicy (`EntryPointNotFoundException`/`DllNotFoundException`) viram `InvalidOperationException` (`FailedToLoadHostpolicy`); existe `AssemblyDependencyResolver.PlatformNotSupported.cs` para `android`, `browser`, `ios`, `tvos` (marcados `[UnsupportedOSPlatform]`).
- Indexa assemblies por nome simples em um `Dictionary<string,string>` `OrdinalIgnoreCase`; **primeira entrada com o mesmo nome vence** (`TryAdd`).

`ResolveAssemblyToPath`: cultura nao neutra -> procura em `_resourceSearchPaths` (`<raiz>/<cultura>/<nome>.dll`); senao busca por nome simples e **so devolve se o arquivo existe** (consistencia com recursos e nativos). `ResolveUnmanagedDllToPath`: se o nome contem separador, usa o diretorio do componente; senao usa `_nativeSearchPaths`, tentando as variacoes de nome (`LibraryNameVariation`).

Limitacoes por desenho, de `docs/design/features/host-component-dependencies-resolution.md`:

- **Frameworks nao sao processados** ("typical plugin scenarios ... will very likely want to share the framework with the app, and thus the ALC ... should fall back to the app for framework assemblies. Since there's no good way to detect which assembly is framework, it's better to not return them").
- `.runtimeconfig.json` nao e lido; probing paths, tfm, host mode e RID fallback sao herdados do app; usa `DOTNET_SHARED_STORE` etc. do processo.
- Perguntas abertas: self-contained + componente de `dotnet build` (sem probing paths do NuGet), nativos retornam apenas diretorios, satelites retornam apenas pasta-base.

### Padrao canonico

```csharp
sealed class PluginLoadContext : AssemblyLoadContext
{
    private readonly AssemblyDependencyResolver _resolver;

    public PluginLoadContext(string pluginPath, bool collectible = false)
        : base(name: Path.GetFileNameWithoutExtension(pluginPath), isCollectible: collectible)
        => _resolver = new AssemblyDependencyResolver(pluginPath);

    protected override Assembly? Load(AssemblyName name)
    {
        // 1. Contrato compartilhado: NUNCA carregue sua propria copia -> cai no Default
        if (name.Name == "Host.Contracts") return null;

        // 2. Dependencias privadas do plugin (deps.json do plugin)
        string? path = _resolver.ResolveAssemblyToPath(name);
        return path is null ? null : LoadFromAssemblyPath(path);   // null => fallback ao Default
    }

    protected override IntPtr LoadUnmanagedDll(string unmanagedDllName)
    {
        string? path = _resolver.ResolveUnmanagedDllToPath(unmanagedDllName);
        return path is null ? IntPtr.Zero : LoadUnmanagedDllFromPath(path);
    }
}

// Host
var alc = new PluginLoadContext(@"plugins\Foo\Foo.dll", collectible: true);
Assembly asm = alc.LoadFromAssemblyPath(@"plugins\Foo\Foo.dll");
Type t = asm.GetTypes().First(x => typeof(IPlugin).IsAssignableFrom(x));   // IPlugin vem do Host.Contracts do Default
var plugin = (IPlugin)Activator.CreateInstance(t)!;
```

Observacoes:

- **Identidade do contrato**: `IPlugin` so converte via cast se `Host.Contracts` existir em um unico contexto. Com `Load` devolvendo `null` para ele, o binder o resolve no Default; se o plugin trouxesse `Host.Contracts.dll` ao lado e o resolvedor o devolvesse, o cast falharia.
- **Mesmo quando o plugin referencia versao *menor***: compativel (arquivo 05), usa a do host. Versao *maior* que a do host: `FileLoadException` (`FUSION_E_APP_DOMAIN_LOCKED`), a menos que o plugin forneca a sua e o contrato seja carregado no plugin (quebra o cast).
- **Passo 3 vs. override**: para forcar a copia privada de uma biblioteca (nao-contrato) que tambem existe no host, o `Load` deve devolve-la antes do fallback (comentario em `customassemblybinder.cpp`: "enables a LoadContext to override assemblies that have been loaded in TPA context by loading a different (or even the same!) version").

### Reflexao contextual

Quando a biblioteca do host (serializadores, DI) cria objetos por nome de tipo, a inferencia de contexto pode apontar para o Default em vez do plugin. A solucao e `AssemblyLoadContext.EnterContextualReflection()` / `CurrentContextualReflectionContext` (`AssemblyLoadContext.cs` ~linhas 565-640; `docs/design/features/AssemblyLoadContext.ContextualReflection.md`):

```csharp
using (alc.EnterContextualReflection())
{
    // Activator.CreateInstance(string,string), Assembly.Load(string), Type.GetType("X, Y")
    // passam a resolver no contexto do plugin, mesmo se o chamador estiver no Default.
    var obj = Activator.CreateInstance("Foo", "Foo.Plugin")!.Unwrap();
}
```

O estado e `AsyncLocal` (`s_asyncLocalCurrent`), entao atravessa `await`. O doc motiva com `Xunit` e `System.Xaml` (nome de tipo qualificado por assembly durante desserializacao).

### Hosting nativo e contextos nomeados

`docs/design/features/native-hosting-load-context.md` estende o padrao a hosts nativos, COM e C++/CLI: `load_assembly`/`get_function_pointer` aceitam um `load_context { identifier }`; componentes com o mesmo `identifier` compartilham um ALC cujo `Load` consulta os `AssemblyDependencyResolver` de cada componente "in precedence order corresponding to the order in which the components were loaded". Exemplo MSBuild:

```xml
<RuntimeHostConfigurationOption Include="System.Runtime.InteropServices.COM.LoadContextIdentifier"
                                Value="MyCompany.MyApp.Plugins" />
```

(Nao-objetivos do doc: unload de componentes e carregar o app inteiro num contexto nao-Default.)

### Plugins que dividem o Default: startup hooks

`docs/design/features/host-startup-hook.md` descreve codigo executado antes do `Main` (variavel `DOTNET_STARTUP_HOOKS`) que pode registrar `Resolving` no Default. E o plugin "global" por excelencia: cuidado, nao tem isolamento algum.

### Testes e exemplos no repo

- `src/tests/Loader/AssemblyDependencyResolver/AssemblyDependencyResolverTests/` (`AssemblyDependencyResolverTests.cs`, `NativeDependencyTests.cs`, `TestBase.cs`) e `MissingHostPolicyTests/InvalidHostingTest.cs` (cenario sem hostpolicy).
- `src/libraries/System.Runtime.Loader/tests/` (`AssemblyLoadContextTest.cs`, `CustomTPALoadContext.cs`, `ContextualReflection.cs`, `DefaultContext/`).
- `docs/design/features/native-hosting.md` e `hosting-layer-apis.md` para quem ainda nao esta em codigo gerenciado.

### Checklist de isolamento (derivado do codigo)

| Pergunta | Mecanismo | Fonte |
|---|---|---|
| Quais tipos devem ser compartilhados? | `Load` devolve `null` -> Default | `customassemblybinder.cpp` passo 3 |
| Como carregar dependencias privadas? | `AssemblyDependencyResolver` | `AssemblyDependencyResolver.cs` |
| E as bibliotecas nativas? | `LoadUnmanagedDll` + `ResolveUnmanagedDllToPath` | arquivo 09 |
| E os recursos localizados? | `ResolveSatelliteAssembly` carrega no ALC do pai | arquivo 06 |
| Como a reflexao encontra o plugin? | `EnterContextualReflection` | `AssemblyLoadContext.cs` |
| Como descarregar? | `isCollectible: true` + `Unload()` | arquivo 08 |
| E se dois plugins querem o mesmo ALC? | `load_context.identifier` (hosting nativo) | `native-hosting-load-context.md` |

## Por que

- **Cada plugin enxerga suas proprias versoes**: dois plugins podem usar versoes diferentes de `Newtonsoft.Json` sem conflito (statics duplicados por contexto).
- **O host controla a fronteira**: so os assemblies "contrato" sao compartilhados (Default); tudo mais e privado. Isso mantem identidade de tipo estavel para interfaces e permite evolucao independente das implementacoes.
- **Reuso do resolvedor do host** (em vez de reimplementar `.deps.json` em C#): comportamento identico ao startup de uma app (RIDs, servicing, assets nativos), mantendo o formato como unica fonte de verdade (`host-component-dependencies-resolution.md`).
- **Nao resolver frameworks por padrao** evita que o plugin carregue um segundo `System.Private.CoreLib`/`System.*`, o que quebraria tudo (CoreLib e proibido fora do Default).
- **Fallback ao Default por padrao** torna o caminho mais simples (plugin ligado ao framework do host) correto "de graca".
- **Contexto como unidade de descarga**: plugins hot-reload (dotnet watch, analyzers) usam coletavel.

## Trade-offs

- **Cast entre fronteiras**: qualquer tipo compartilhado precisa vir do Default; se o plugin empacotar sua propria copia do contrato, os tipos divergem. Erros aparecem apenas em tempo de execucao (`InvalidCastException`, `MissingMethodException`).
- **Versoes do contrato**: o host precisa manter compatibilidade binaria; plugin compilado contra contrato mais novo falha ao carregar.
- **Dependencia transitiva compartilhada acidentalmente**: se o host e o plugin usam `System.Text.Json` e o plugin traz outra versao, o plugin pode receber a do host no passo 3 (fallback), a menos que `Load` a devolva.
- **Memoria e JIT**: cada plugin carrega suas copias; R2R so na primeira por caminho.
- **Reflexao contextual** e complexa e dependente de `AsyncLocal`; esquecer o scope causa carga no contexto errado.
- **Nativos nao sao isolados**: bibliotecas nativas com o mesmo nome compartilham o espaco de enderecos do processo; duas versoes da mesma `.so` podem colidir em simbolos; handles nao sao liberados no `Unload`.
- **Descarregamento difícil** (arquivo 08), e IJW/R2R nao funcionam em coletaveis.
- **`AssemblyDependencyResolver` indisponivel** em algumas plataformas (android/browser/ios/tvos) e exige hostpolicy presente (`InvalidHostingTest.cs` cobre o erro).

## Como aplicar na sua linguagem

1. **Desenhe o plugin como "escopo + manifesto + contrato"**: um escopo de carga por plugin, um manifesto proprio (como o `deps.json` do plugin) e uma lista explicita de **modulos de contrato** compartilhados com o host.
2. **Torne o "pai" explicito e o compartilhamento por lista**: em vez de herdar tudo do pai, permita `shared = ["Host.Contracts", "stdlib"]`; qualquer modulo fora da lista e privado. O .NET faz o inverso (privado so se `Load` devolver); a abordagem por lista previne o erro mais comum.
3. **Reaproveite o resolvedor do launcher** para plugins: exponha `ComponentResolver::new(path)` que usa a mesma biblioteca que resolve o app. Evite duas implementacoes.
4. **Exclua a plataforma do manifesto do plugin por padrao** (como frameworks no `AssemblyDependencyResolver`) e permita opt-in para ferramentas de analise (MetadataLoadContext).
5. **Forneca "escopo corrente" dinamico** (`with plugin_scope:`) para APIs de carga por string.
6. **Verifique identidade de contrato no carregamento**: ao carregar um plugin, compare a versao do modulo de contrato esperado com a do host e falhe com mensagem clara em vez de `InvalidCast`.
7. **Trate nativos como compartilhados**: documente que nao ha isolamento; considere carregar extensoes nativas em processo separado quando isolamento forte for requisito.
8. **Permita escopos nomeados para plugins cooperantes** (como `load_context.identifier`), com um mapa global e aviso para prefixar o nome.
9. **Combine com coletavel** somente se a linguagem suportar o protocolo do arquivo 08; ofereca `Unloading` event e ferramentas para achar raizes retentoras.
10. **Teste com o harness do repo como modelo**: plugin com dependencia privada, plugin com dependencia compartilhada de versao diferente, plugin com nativo, plugin descarregado.

## Referencias

- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`, `AssemblyDependencyResolver.PlatformNotSupported.cs`, `AssemblyLoadContext.cs`, `LibraryNameVariation*.cs`
- `src/native/corehost/hostpolicy/hostpolicy.cpp` (`corehost_resolve_component_dependencies`, `corehost_set_error_writer`)
- `src/coreclr/binder/customassemblybinder.cpp`
- `docs/design/features/host-component-dependencies-resolution.md`, `native-hosting-load-context.md`, `native-hosting.md`, `hosting-layer-apis.md`, `AssemblyLoadContext.ContextualReflection.md`, `host-startup-hook.md`, `assemblyloadcontext.md`, `unloadability.md`
- `src/tests/Loader/AssemblyDependencyResolver/`, `src/libraries/System.Runtime.Loader/tests/`
- `src/libraries/System.Reflection.MetadataLoadContext/` (carga somente para inspecao, sem execucao)
- ECMA-335 nao define plugins nem isolamento de carga; o conceito historico mais proximo (AppDomain) nao existe no .NET Core alem do dominio unico.
