# Satellite assemblies e recursos localizados

> Objetivo: documentar como cultura vira um eixo de resolucao de modulos: o contrato de nomes (`<Nome>.resources`), a ordem de busca no binder, o ramo gerenciado em `AssemblyLoadContext` e o consumidor (`ResourceManager`).

## Mecanismo no .NET

### Convencao de nomes e layout

Um *satellite assembly* contem apenas recursos (sem codigo) para uma cultura. Pelo codigo:

- Nome simples: `<NomeDoAssemblyPrincipal>.resources` (`const string SatelliteSuffix = ".resources"` em `AssemblyLoadContext.cs`, metodo `ResolveSatelliteAssembly`).
- Cultura: a do `AssemblyName.CultureName` (por exemplo `pt-BR`).
- Arquivo em disco: `<diretorio-do-principal>/<cultura>/<Nome>.resources.dll`. O binder nativo monta `cultura/nome.dll` (`BindSatelliteResource`: `CombinePath(fileName, cultureRef...)`, `CombinePath(fileName, simpleNameRef...)`, `Append(".dll")`). Note que `simpleNameRef` ja inclui `.resources`.
- Cultura neutra (`neutral` ou vazia) significa assembly de codigo; qualquer outra dispara o ramo de satelite (`IsNeutralCulture()` em `BindByTpaList`).

Os recursos "padrao" (cultura neutra do app) normalmente ficam embutidos no assembly principal; `NeutralResourcesLanguageAttribute` diz qual cultura eles representam e `UltimateResourceFallbackLocation` (`MainAssembly` ou `Satellite`) diz onde procurar o fallback final (`ManifestBasedResourceGroveler.GetNeutralResourcesLanguage`, `src/libraries/System.Private.CoreLib/src/System/Resources/ManifestBasedResourceGroveler.cs`, ~linha 126).

### Quem pede um satelite

`ResourceManager` (`src/libraries/System.Private.CoreLib/src/System/Resources/`) usa `ManifestBasedResourceGroveler`:

1. `UltimateFallbackFixup(culture)`: ajusta a cultura final.
2. Se a cultura e a invariante e o fallback esta no `MainAssembly`, nem procura satelites.
3. Caso contrario `GetSatelliteAssembly(lookForCulture)` -> `InternalGetSatelliteAssembly(mainAssembly, culture, version, throwOnFileNotFound: false)` -> `RuntimeAssembly.InternalGetSatelliteAssembly`, (`src/coreclr/System.Private.CoreLib/src/System/Reflection/RuntimeAssembly.cs`, ~linha 662). Ele monta um `AssemblyName` com: chave publica e flags copiados do principal, `Version = version ?? principal.GetVersion()`, `CultureInfo = culture` e `Name = <nome simples do principal> + ".resources"`, e chama `InternalLoad(an, requestingAssembly: principal, ...)`. Como o *requestingAssembly* e o principal, o bind ocorre no **contexto do assembly principal** (regra do solicitante, arquivo 01). Se o resultado for o proprio principal, devolve `null`.

   Observe que a chave publica ainda e copiada para a referencia, embora o binder nao a compare por padrao (arquivo 05): resquicio do modelo strong-name.
4. Se faltar e a cultura invariante exigir satelite -> `HandleSatelliteMissing()` (excecao `MissingSatelliteAssemblyException`).
5. Abre o stream do manifesto e instancia o `ResourceSet`.

A cadeia de fallback de cultura (`pt-BR` -> `pt` -> invariante) e feita pelo `ResourceFallbackManager`, nao pelo binder. O binder responde "tenho/nao tenho esta cultura exata".

### Passo nativo: `BindSatelliteResource`

`src/coreclr/binder/assemblybindercommon.cpp` (~linha 698). O comentario descreve a estrategia:

```
1) Probe extensions (single-file bundle / dados externos)       <- AssemblyProbeExtension::Probe(relativePath)
2) Para cada Platform Resource Root (PLATFORM_RESOURCE_ROOTS)    <- BindSatelliteResourceByProbingPaths
3) Para cada App Path (APP_PATHS)
Durante cada busca, se achar arquivo com o mesmo nome mas ref/def nao casa,
cai para a busca de recursos do app ("handle case where a user creates resources with the same names as platform ones").
```

Detalhes de `BindSatelliteResourceByProbingPaths`: para cada raiz concatena `raiz/cultura/nome.dll`, chama `GetAssembly(..., fIsInTPA=FALSE)`, registra `BinderTracing::PathProbed`; `ERROR_FILE_NOT_FOUND` e esperado e leva a proxima raiz; se o arquivo existe, valida com `TestCandidateRefMatchesDef(..., tpaListAssembly=false)` e devolve `FUSION_E_REF_DEF_MISMATCH` se nao casar. Se nenhuma raiz tem o arquivo, devolve `S_OK` sem resultado ("Up-stack expects S_OK when we don't find any candidate"), o que faz o chamador seguir para os ramos gerenciados.

Os satelites **nunca estao na TPA** (`fIsInTPA=FALSE`): a TPA e por nome simples, e o mesmo nome (`Foo.resources`) existe uma vez por cultura. As raizes vem do host: `resolve_probe_dirs(deps_entry_t::asset_types::resources, ...)` em `deps_resolver.cpp` (que, para `resources`, usa o diretorio-base removendo a cultura) e a propriedade `PLATFORM_RESOURCE_ROOTS`.

### Passo gerenciado: `ResolveSatelliteAssembly`

Se o nativo nao resolve (Default ou Custom), o runtime chama `AssemblyLoadContext.ResolveSatelliteAssembly` (callback `[UnmanagedCallersOnly]` em `AssemblyLoadContext.cs`). A implementacao, em essencia:

```csharp
// so aceita nomes terminados em ".resources"
string parent = name.Name[..^".resources".Length];
RuntimeAssembly parentAssembly = (RuntimeAssembly)LoadFromAssemblyName(new AssemblyName(parent));
AssemblyLoadContext parentALC = GetLoadContext(parentAssembly)!;
string? dir = Path.GetDirectoryName(parentAssembly.Location);
string path = Path.Combine(dir, name.CultureName!, $"{name.Name}.dll");
bool exists = FileSystem.FileExists(path);
if (!exists && PathInternal.IsCaseSensitive)           // Linux: tenta cultura em minusculas
    path = Path.Combine(dir, name.CultureName!.ToLowerInvariant(), $"{name.Name}.dll");
return exists ? parentALC.LoadFromAssemblyPath(path) : null;   // carrega NO ALC DO PAI
```

Pontos importantes:

- O satelite e carregado **no ALC do assembly principal** (`parentALC`), nao no ALC que pediu. Isso mantem o par principal/satelite no mesmo contexto.
- Existe `TraceSatelliteSubdirectoryPathProbed` para depuracao (EventPipe).
- Em `TARGET_BROWSER` o caminho e virtual (`/{cultura}/{nome}.dll`), porque assemblies carregados por probe externo tem `Location` vazio.
- Para o **Default**, o objeto gerenciado so e criado quando um satelite e pedido (ver `DefaultAssemblyBinder::BindUsingAssemblyName`, que chama `INITIALIZE_DEFAULT_CONTEXT`): "For satellite assemblies, the managed ALC has additional resolution logic (defined by the runtime) which should be run even if the managed default ALC has not yet been used".
- Para ALC customizado, a ordem e (do comentario de `customassemblybinder.cpp`) `Load` -> Default (exceto satelites) -> `ResolveSatelliteAssembly` -> `Resolving`. Pedidos de satelite **pulam** o fallback ao Default, porque o satelite correto esta ao lado do assembly principal, que pode ser do plugin.

### Satelites em plugins: `AssemblyDependencyResolver`

`AssemblyDependencyResolver.ResolveAssemblyToPath` (`src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`) separa o caso: se `CultureName` nao e vazio nem `"neutral"`, procura em `_resourceSearchPaths` combinando `searchPath / cultura / nome.dll`; caso contrario consulta o dicionario de codigo. O comentario diz que replica `BindSatelliteResourceByResourceRoots` e que **nao** replica o probing de `APP_PATHS` ("effectively being deprecated"). O hostpolicy devolve para recursos apenas a pasta-base (`./foo/en-us/resource.dll` -> `./foo`), conforme `host-component-dependencies-resolution.md`.

### Single-file

Em bundles, o satelite e procurado primeiro no manifesto do bundle (`BindSatelliteResourceByProbeExtension`, `Bundle::AppIsBundle`), com `PathSource::Bundle` no trace; o comentario em `AssemblyLoadContext.cs` observa que "native call runs before this and checks BindSatelliteResourceFromBundle".

### Exemplo completo

```
MyApp/
  MyApp.dll                       (recursos neutros embutidos; [NeutralResourcesLanguage("en")])
  MyApp.deps.json                 (asset "resources" por cultura)
  pt-BR/MyApp.resources.dll
  fr/MyApp.resources.dll
```

```csharp
var rm = new ResourceManager("MyApp.Strings", typeof(Program).Assembly);
CultureInfo.CurrentUICulture = new CultureInfo("pt-BR");
Console.WriteLine(rm.GetString("Hello"));
// 1. ResourceManager pede MyApp.resources, Culture=pt-BR
// 2. Binder nativo: bundle -> PLATFORM_RESOURCE_ROOTS -> APP_PATHS
// 3. ALC.ResolveSatelliteAssembly: <dir de MyApp.dll>/pt-BR/MyApp.resources.dll
// 4. Se faltar, ResourceFallbackManager tenta "pt" e depois a invariante (MyApp.dll)
```

### Tabela: quem resolve o que

| Etapa | Componente | Arquivo |
|---|---|---|
| Escolher cultura-alvo e cadeia de fallback | `ResourceManager` / `ResourceFallbackManager` | `src/libraries/System.Private.CoreLib/src/System/Resources/` |
| Montar a referencia `<nome>.resources, Culture=x` | `RuntimeAssembly.InternalGetSatelliteAssembly` | `src/coreclr/System.Private.CoreLib/src/System/Reflection/RuntimeAssembly.cs` |
| Procurar em bundle, resource roots, app paths | `BindSatelliteResource` | `src/coreclr/binder/assemblybindercommon.cpp` |
| Procurar ao lado do assembly pai | `AssemblyLoadContext.ResolveSatelliteAssembly` | `AssemblyLoadContext.cs` |
| Informar as raizes de recursos | hostpolicy (`PLATFORM_RESOURCE_ROOTS`) | `src/native/corehost/hostpolicy/deps_resolver.cpp`, `coreclr.cpp` |
| Resolver para plugin | `AssemblyDependencyResolver` | `.../Runtime/Loader/AssemblyDependencyResolver.cs` |

### Casos de borda observados no codigo

- **Satelite do proprio principal**: `InternalGetSatelliteAssembly` devolve `null` se o bind resolver para o proprio assembly principal (evita auto-referencia).
- **Culture `neutral`**: `AssemblyDependencyResolver` trata `"neutral"` como nao-satelite (comparacao `OrdinalIgnoreCase`).
- **`parentDirectory == null`** (assembly sem `Location`, por exemplo carregado de stream): `ResolveSatelliteAssembly` devolve `null`; o satelite so pode ser achado pelos ramos nativos (bundle) ou por um handler de `Resolving`.
- **Carregamento por stream**: um plugin carregado com `LoadFromStream` nao tem diretorio; os satelites precisam ser fornecidos pelo `Resolving`/`Load` customizado.
- **Falha cacheada**: cada par (`Foo.resources`, cultura) ausente entra no `FailureCache` do `ApplicationContext`, evitando repetir IO em chamadas seguintes.

## Por que

- **Localizacao sem recompilar**: separar recursos em assemblies independentes permite entregar traducoes novas sem tocar no codigo, e carregar so a cultura necessaria (economia de memoria/disco no startup).
- **Cultura na identidade** e o gancho que deixa o binder tratar satelites como "mesmo nome, outra cultura", sem tabela propria. O custo e um ramo especial em `BindByTpaList`.
- **Fora da TPA**: a TPA e O(1) por nome; para satelites a chave seria (nome, cultura), entao o design optou por *probing em diretorios* de raizes de recursos.
- **Ramo gerenciado**: manter a regra `<dir>/<cultura>/<nome>.dll` em C# permite que a logica seja a mesma para Default e custom, e que plugins herdem o comportamento sem implementar `Load`.
- **Carregar no ALC do pai**: preserva o isolamento (o plugin e seus recursos formam uma unidade) e evita que um satelite do plugin vaze para o Default.
- **Tratar o caso Linux** (`IsCaseSensitive`): culturas como `pt-BR` vs `pt-br` variam entre ferramentas de build e SOs; o fallback para minusculas evita falhas por caixa.

## Trade-offs

- **Dois sistemas de resolucao** (nativo para raizes + gerenciado para dir. do pai) com semantica quase igual: duplicacao e risco de divergencia (os comentarios do proprio resolvedor dizem "same logic as BindSatelliteResourceByResourceRoots").
- **IO por tentativa**: cada cultura da cadeia de fallback pode custar varios `File.Exists`; por isso o cache de falhas do `ApplicationContext` e relevante.
- **Satelites ignoram a TPA e o manifesto**: dependem de layout em disco/bundle, o que complica trimming e AOT.
- **Cultura como parte do nome** (`.resources` + cultura) mistura localizacao com identidade de modulo; modelos modernos preferem catalogos de mensagens (ex.: arquivos por locale) fora do sistema de assemblies.
- **Estado global implicito**: `CurrentUICulture` determina qual satelite carregar, entao o assembly carregado depende de estado de thread.
- **Compatibilidade de versao**: o satelite deve casar com a versao do principal (`InternalGetSatelliteAssembly(..., Version? version)`), ou o `TestCandidateRefMatchesDef` falha com `REF_DEF_MISMATCH` (ver arquivo 05).

## Como aplicar na sua linguagem

1. **Decida se recursos localizados sao modulos.** Se sim, defina um *tipo de modulo* "recursos" (como o asset `resources` do `deps.json`) com `(modulo-pai, cultura)` na identidade e um caminho convencional `<pai>/<cultura>/<nome>.res`. Se nao, use catalogos de dados carregados sob demanda por uma API de i18n e mantenha o resolvedor de modulos livre de cultura.
2. **Mantenha o satelite no mesmo escopo de carga do modulo pai** (como `parentALC.LoadFromAssemblyPath`), nunca no escopo do requisitante.
3. **Separe resolucao de fallback de cultura** (cadeia `pt-BR -> pt -> invariante`) da resolucao de arquivo; o fallback pertence ao consumidor de recursos, nao ao binder.
4. **Normalize caixa de cultura** na identidade (use `ToLowerInvariant` no layout do disco) e tente a variante exata primeiro, como `ResolveSatelliteAssembly` faz.
5. **Nao ponha satelites no mapa de modulos da plataforma**; use raizes de recursos configuradas pelo host e cache de ausencia.
6. **Pule o fallback ao escopo pai para satelites** (o recurso correto esta ao lado do modulo que o pediu).
7. **Defina cultura neutra de origem em metadados** (`NeutralResourcesLanguage`) e onde fica o fallback final, para evitar probing inutil da cultura de desenvolvimento.
8. **Planeje single-file/AOT desde o inicio**: preveja um "probe de bundle" para recursos (como `AssemblyProbeExtension`).

## Referencias

- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyLoadContext.cs` (`ResolveSatelliteAssembly`)
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`
- `src/libraries/System.Private.CoreLib/src/System/Resources/ManifestBasedResourceGroveler.cs`, `ResourceManager.cs`, `ResourceFallbackManager.cs`
- `src/coreclr/binder/assemblybindercommon.cpp` (`BindSatelliteResource*`, `BindByTpaList`, `BindToSystemSatellite`)
- `src/coreclr/binder/defaultassemblybinder.cpp`, `customassemblybinder.cpp`
- `src/native/corehost/hostpolicy/deps_resolver.cpp` (`resolve_probe_dirs`)
- `docs/design/features/host-component-dependencies-resolution.md`, `host-probing.md`
- ECMA-335 Partition II 6.2.1 (cultura em `Assembly`), 22.2 (campo `Culture` da tabela Assembly) e a descricao de recursos em Partition II 6.2.2 (manifest resources). A convencao `.resources` e do .NET, nao da ECMA.
