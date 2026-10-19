# Da restauracao NuGet ao runtime: a ponte do grafo de pacotes

> Objetivo: mapear a cadeia de artefatos que transforma um grafo de pacotes resolvido em tempo de build (NuGet) em uma lista de arquivos carregaveis em tempo de execucao. Parte da cadeia vive no SDK (fora deste repositorio) e parte aqui; este documento diz explicitamente qual e qual.

## Mecanismo no .NET

### A cadeia, de ponta a ponta

```
*.csproj (PackageReference, FrameworkReference, RuntimeIdentifier)
   |  NuGet restore                         [NuGet + .NET SDK, fora deste repo]
   v
obj/project.assets.json                     (grafo completo: targets por TFM/RID, libraries, assets)
   |  SDK: ResolvePackageAssets / GenerateDepsFile / GenerateRuntimeConfigurationFiles  [fora deste repo]
   v
bin/app.deps.json          bin/app.runtimeconfig.json
   |                                |
   |  host (src/native/corehost)    |
   v                                v
hostfxr (escolhe framework)  --->  hostpolicy (deps_resolver_t)  --->  TPA / native dirs / resource roots
                                                                         |
                                                                         v
                                                          coreclr_initialize -> binder (src/coreclr/binder)
```

Neste repositorio estao os **consumidores** (host, binder, `AssemblyDependencyResolver`) e os **produtores dos pacotes de plataforma** (`src/installer/pkg/sfx/Microsoft.NETCore.App/` com projetos `*.sfxproj` para `Ref`, `Runtime.CoreCLR`, `Runtime.Mono`, `Runtime.NativeAOT`, `Crossgen2`). Os alvos MSBuild que geram `deps.json` a partir de `project.assets.json` pertencem ao .NET SDK e nao estao aqui (uma busca por `GenerateDepsFile` no repo so encontra usos em `.props` de ferramentas e projetos de teste, por exemplo `src/coreclr/tools/aot/ILCompiler/ILCompiler.props` e `src/installer/tests/Assets/Projects/ComLibrary/ComLibrary.csproj`).

### Tres arquivos, tres papeis

| Arquivo | Escrito por | Lido por | Conteudo-chave |
|---|---|---|---|
| `project.assets.json` | NuGet restore | SDK (build) | grafo completo, incluindo assets de compilacao |
| `*.deps.json` | SDK | hostpolicy, `DependencyModel` | **so o necessario para executar**: `targets`, `libraries`, `runtimes` (fallback de RID), `serviceable`, `path`, `sha512`, versoes de assembly/arquivo |
| `*.runtimeconfig.json` | SDK | hostfxr/hostpolicy | `tfm`, `frameworks` (nome, versao minima, `rollForward`), `configProperties` (viram `AppContext` switches), `additionalProbingPaths` |

`docs/design/features/framework-version-resolution.md` define a referencia de framework: **nome + versao minima + roll-forward** (`LatestPatch`, `Minor` (padrao), `Major`, `LatestMinor`, `LatestMajor`, `Disable`), com precedencia `runtimeconfig.json (global) < framework property < DOTNET_ROLL_FORWARD < --roll-forward`. Os frameworks formam um grafo (o app referencia FX1, que referencia FX2...) resolvido em `src/native/corehost/fx_reference.cpp`/`fx_definition.cpp`/`fxr/`. Sem heranca de `rollForward` entre elos da cadeia.

### Como o `deps.json` representa o grafo

Trecho minimo, do `host-probing.md`, mostrando as duas secoes que o host usa:

```json
{
  "runtimeTarget": { "name": ".NETCoreApp,Version=v8.0/win-x64" },
  "targets": {
    ".NETCoreApp,Version=v8.0/win-x64": {
      "Newtonsoft.Json/13.0.3": {
        "runtime":   { "lib/net6.0/Newtonsoft.Json.dll": { "assemblyVersion": "13.0.0.0", "fileVersion": "13.0.3.27908" } },
        "resources": { "lib/net6.0/fr/Newtonsoft.Json.resources.dll": { "locale": "fr" } },
        "runtimeTargets": {
          "runtimes/win-x64/native/libfoo.dll": { "rid": "win-x64", "assetType": "native" }
        }
      }
    }
  },
  "libraries": {
    "Newtonsoft.Json/13.0.3": { "type": "package", "serviceable": true, "sha512": "sha512-...", "path": "newtonsoft.json/13.0.3" }
  }
}
```

(Exemplo ilustrativo no formato do SDK. O que `deps_format.cpp` comprovadamente le: `runtimeTarget`, `libraries[].serviceable`, os assets `runtime`/`resources`/`native` por pacote, a secao `runtimeTargets` (com `assetType` e `rid`, usada para assets especificos de RID, linhas ~384-420), e por asset `assemblyVersion`, `fileVersion` e `localPath`. Campos como `sha512` e `locale` fazem parte do formato do SDK, mas nao os verifiquei no parser do host.) O grafo de **dependencias entre pacotes** (secao `dependencies` por library, no formato do SDK) nao e usado pelo parser do hostpolicy (`deps_format.cpp` nao referencia essa chave): ele itera `libraries` e `targets`, que o SDK ja achatou. A ponte, portanto, e um **achatamento em build-time**: transforma um grafo em uma lista ordenada.

### Tabela: onde cada decisao e tomada

| Decisao | Quando | Quem |
|---|---|---|
| Versao de cada pacote (transitivos) | restore | NuGet |
| Quais assets de runtime entram | build/publish | SDK (gera `deps.json`) |
| Qual framework instalado satisfaz a referencia | startup | `hostfxr` (roll-forward) |
| Qual arquivo fisico corresponde a cada asset | startup | `hostpolicy` (probing) |
| Qual assembly satisfaz uma referencia por nome | execucao | binder CoreCLR (arquivos 04, 05) |
| Qual contexto o carrega | execucao | `AssemblyLoadContext` (arquivos 01, 02) |

### Selecao de assets por plataforma

O host precisa escolher, dentro do pacote, os assets nativos certos. Isso e feito em `deps_format.cpp`: `get_current_machine_rid`, `try_get_matching_rid`, `try_get_matching_rid_with_fallback_graph`. O grafo `runtimes` (fallback, por exemplo `win10-x64 -> win-x64 -> win -> any`) so e lido se `System.Runtime.Loader.UseRidGraph` for `true` (`hostpolicy_context.cpp::should_read_rid_fallback_graph`); por padrao e desativado e o host usa uma lista fixa de RIDs conhecidos (`s_host_rids`) mais o RID da variavel de ambiente. Para apps self-contained o SDK ja resolve o RID e grava somente assets do RID alvo.

### O app e `copy-local`

Desde o .NET Core 3.0, `dotnet build` copia dependencias estaticas para a pasta de saida (`host-component-dependencies-resolution.md`: "with the changes in .NET Core 3 where `dotnet build` will copy all static dependencies locally, the importance of additional probing paths..."), ou seja, na pratica `bin/` ja contem o que o `deps.json` lista. Os probing paths do NuGet cache (`~/.nuget/packages`) ficam como fallback (`additionalProbingPaths` em `.runtimeconfig.dev.json`, gerado em Debug).

### O framework tambem tem seu `deps.json`

`Microsoft.NETCore.App.deps.json` (produzido por `src/installer/pkg/sfx/Microsoft.NETCore.App/*.sfxproj`) lista os assemblies do runtime pack. Na ordem de processamento (app > additional-deps > frameworks de alto para baixo; `assembly-conflict-resolution.md`) ele e o ultimo e menos prioritario, mas se contiver versao maior do mesmo assembly, substitui a do app.

### Light-up e shared store

- `--additional-deps` / `DOTNET_ADDITIONAL_DEPS` (`docs/design/features/additional-deps.md`): deps.json extras, tipicamente de "extensoes light-up" (por exemplo, o Application Insights do Azure App Service), processados logo depois do app. Pastas seguem `shared/<framework_name>/<requested_framework_version>`.
- **Shared store** (`DotNetCore-SharedPackageStore.md`): pasta de pacotes pre-instalados em layout de cache NuGet (`store/<arch>/<tfm>/<id>/<versao>`), que permite "trimar" o app das dependencias compartilhadas. Entra na lista de probing (`DOTNET_SHARED_STORE`). E um mecanismo legado, pouco usado hoje.

### Leitura em tempo de execucao: `Microsoft.Extensions.DependencyModel`

`src/libraries/Microsoft.Extensions.DependencyModel/src/` expoe o mesmo arquivo como modelo de objetos: `DependencyContext` (`DependencyContext.Default`), `DependencyContextJsonReader`/`Writer`, `RuntimeLibrary`, `RuntimeAssetGroup`, `RuntimeFallbacks`, `ResourceAssembly`, mais `Resolution/` (resolvedores de caminho de compilacao). Diferencas frente ao hostpolicy, enumeradas em `host-component-dependencies-resolution.md`: o DependencyModel descreve o arquivo inteiro (inclusive assets de compilacao), nao faz servicing/store/RID fallback "como o host" e traz sua propria dependencia JSON; o hostpolicy so olha assets de runtime do RID atual.

```csharp
var ctx = DependencyContext.Default!;                  // le o deps.json do app
foreach (RuntimeLibrary lib in ctx.RuntimeLibraries)
    foreach (var group in lib.RuntimeAssemblyGroups)
        Console.WriteLine($"{lib.Name} {lib.Version}: {string.Join(",", group.AssetPaths)}");
```

### Plugins: reaproveitando a ponte

Para um plugin com seu proprio `deps.json`, `AssemblyDependencyResolver` (arquivo 11) chama `corehost_resolve_component_dependencies` (`src/native/corehost/hostpolicy/hostpolicy.cpp`) e recebe tres listas. **Nao processa frameworks nem `runtimeconfig.json`**, ou seja, assume que a plataforma vem do host. Limitacao conhecida: os probing paths sao os do app, entao um app self-contained nao encontra pacotes do cache NuGet usados por um componente.

### Hosting nativo: a ponte pode ser contornada

`hostfxr`/`hostpolicy` podem ser inicializados por um host proprio (`docs/design/features/native-hosting.md`, `hosting-layer-apis.md`); o host pode pular a ponte e definir `TRUSTED_PLATFORM_ASSEMBLIES` diretamente. O contrato `host_runtime_contract.h` tambem permite entregar assemblies sem arquivos (`external_assembly_probe`) e sobrescrever P/Invokes (`pinvoke_override`).

## Por que

- **Mover trabalho caro para build-time**: NuGet resolve versoes, dependencias transitivas e compatibilidade de TFM uma vez, no restore. O host so le uma lista. O startup fica previsivel e sem acesso a rede.
- **Separar "o que e necessario para compilar" de "para rodar"**: `project.assets.json` guarda ambos; o `deps.json` filtra assets de runtime. Reduz tamanho e superficie de falha no host.
- **Frameworks como referencia nominal**: o app declara "preciso de `Microsoft.NETCore.App >= 8.0.0` com roll-forward `Minor`" e o host escolhe a instalacao; assim patches de seguranca chegam sem recompilar.
- **Duas camadas de arquivo** (`deps.json` x `runtimeconfig.json`): grafo de pacotes versus politica de execucao; permite editar politica (roll-forward, GC settings) sem alterar o grafo.
- **Host em C++ sem dependencias**: o hostpolicy usa um parser JSON nativo (`json_parser.cpp`, baseado em rapidjson) e nao depende de NuGet nem de .NET gerenciado; essencial porque roda antes de existir um runtime gerenciado.
- **RID fallback desligado por padrao**: simplifica; o SDK moderno ja grava os RIDs finais.

## Trade-offs

- **Dois mundos sincronizados por contrato de arquivo**: mudar o formato do `deps.json` exige coordenacao SDK/host/`DependencyModel`.
- **Complexidade herdada**: servicing, shared store, `additional-deps`, `probing paths` e `.runtimeconfig.dev.json` existem por razoes historicas e ainda tem de ser suportados.
- **Perde-se o grafo** no host: o host nao sabe quem depende de quem; resolver conflitos so por versao/ordem.
- **Dependencia de ferramental**: sem SDK, produzir `deps.json` a mao e trabalhoso; apps "soltos" caem em heuristicas.
- **Verificabilidade**: nao encontrei no parser do host (`deps_format.cpp`) nenhuma leitura de hash de conteudo; o campo `sha512` do SDK e, no que se pode confirmar neste repo, ignorado em execucao. Nao conte com o `deps.json` como garantia de integridade.
- **`DependencyModel` duplica logica do host**, com semanticas parcialmente diferentes.

## Como aplicar na sua linguagem

1. **Separe tres fases/artefatos** desde o dia 1: (a) resolucao de versoes/dependencias (gerenciador de pacotes, produz *lockfile*), (b) um **manifesto de execucao achatado** (equivalente ao `deps.json`: lista ordenada de modulos com caminho relativo, versao, tipo de asset e plataforma), (c) uma **politica de execucao** (equivalente ao `runtimeconfig.json`: qual runtime/plataforma, versao minima, regra de roll-forward, flags).
2. **Achate o grafo em build/publish.** O runtime nao deve percorrer arestas de dependencia nem consultar o registry; deve apenas ler a lista.
3. **Modele a plataforma como "framework reference"**: nome + versao minima + politica de roll-forward com enumeracao fechada (copie a tabela do .NET: LatestPatch/Minor/Major/Latest*/Disable) e precedencia clara config < env < CLI.
4. **Gere um manifesto para a propria plataforma** e processe-o por ultimo, com regra "maior versao vence".
5. **Inclua hash de conteudo no lockfile e verifique no restore**, e opcionalmente no carregamento em modo "seguro" (o .NET nao verifica no host).
6. **Dê uma API de leitura** do manifesto em tempo de execucao (`DependencyContext`), mas deixe claro em que difere da resolucao real do host; idealmente exponha o *mesmo* codigo (como `corehost_resolve_component_dependencies`) para evitar divergencia.
7. **Gere assets nativos por RID/plataforma ja no manifesto** (nao faca fallback de RID em runtime) a menos que precise.
8. **Permita bypass para embedders**: um modo em que o hospedeiro entrega os caminhos diretamente (como `TRUSTED_PLATFORM_ASSEMBLIES` ou o contrato de callbacks).
9. **Decida cedo o que copiar local** (layout "app-local" = reprodutivel) vs cache global compartilhado; o .NET migrou de store/cache para copy-local por previsibilidade.

## Referencias

- `docs/design/features/framework-version-resolution.md`, `sharedfx-lookup.md`, `roll-forward-on-no-candidate-fx.md`
- `docs/design/features/additional-deps.md`, `DotNetCore-SharedPackageStore.md`
- `docs/design/features/host-probing.md`, `host-component-dependencies-resolution.md`, `native-hosting.md`, `hosting-layer-apis.md`
- `src/native/corehost/hostpolicy/deps_format.cpp`, `deps_resolver.cpp`, `hostpolicy_context.cpp`, `hostpolicy.cpp`
- `src/native/corehost/fx_reference.cpp`, `fx_definition.cpp`, `runtime_config.cpp`, `roll_forward_option.cpp`, `json_parser.cpp`, `fxr/`
- `src/native/corehost/host_runtime_contract.h`
- `src/libraries/Microsoft.Extensions.DependencyModel/src/` (`DependencyContext.cs`, `DependencyContextJsonReader.cs`, `RuntimeLibrary.cs`, `RuntimeFallbacks.cs`)
- `src/installer/pkg/sfx/Microsoft.NETCore.App/` (empacotamento do framework e do runtime pack)
- Fora do repo: NuGet (`project.assets.json`), .NET SDK (`Microsoft.NET.Sdk.targets`, `ResolvePackageAssets`, `GenerateDepsFile`).
- ECMA-335 nao cobre nada disso (a CLI nao define empacotamento nem restauracao).
