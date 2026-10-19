# Resolucao de assembly: probing paths e deps.json

> Objetivo: documentar como o *host* (hostfxr/hostpolicy) transforma um manifesto (`.deps.json` + `.runtimeconfig.json`) em caminhos de arquivo concretos que o runtime consome, e como conflitos entre app e framework sao resolvidos.

## Mecanismo no .NET

### Divisao de responsabilidades

O .NET separa **resolucao de arquivos** (host) de **binding por identidade** (runtime):

- O **host** (`src/native/corehost/`) le `*.runtimeconfig.json` e `*.deps.json`, escolhe a versao do framework (roll-forward), percorre os *probing paths* e produz tres listas: assemblies (TPA), diretorios de busca de nativos e raizes de recursos.
- O **runtime** (`src/coreclr/binder/`) recebe essas listas como *propriedades* e faz o bind por nome (arquivos 04 e 05).

Componentes do host (`docs/design/features/host-components.md`): `apphost`/`dotnet` (muxer) -> `hostfxr` (resolve SDK/framework, le config) -> `hostpolicy` (le `deps.json`, monta propriedades, inicia o CoreCLR). Codigo relevante em `src/native/corehost/`: `fxr/`, `hostpolicy/`, `hostcommon/`, `hostmisc/`.

### Anatomia do `.deps.json`

`docs/design/features/host-probing.md` usa este exemplo (apenas partes relevantes):

```json
{
  "targets": {
    ".NETCoreApp,Version=v2.1": {
      "Newtonsoft.Json/11.0.2": {
        "runtime": { "lib/netstandard2.0/Newtonsoft.Json.dll": {} }
      }
    }
  },
  "libraries": {
    "Newtonsoft.Json/11.0.2": {
      "type": "package",
      "serviceable": true,
      "path": "newtonsoft.json/11.0.2"
    }
  }
}
```

Dois caminhos relativos se combinam: o do **library** (`path`) e o do **asset** (`lib/netstandard2.0/Newtonsoft.Json.dll`). O parser esta em `src/native/corehost/hostpolicy/deps_format.cpp`; o tipo de asset e um enum de tres valores (`runtime`, `resources`, `native`, declarado no topo do arquivo). Alem de `targets`/`libraries`, o parser le `runtimes` (grafo de fallback de RID) e, por asset, `assemblyVersion` e `fileVersion` (usados na resolucao de conflitos; ver abaixo).

Ponto importante: **RID e selecao de assets nativos**. `deps_format.cpp` (`try_get_matching_rid`, `try_get_matching_rid_with_fallback_graph`, `get_current_machine_rid`) escolhe o RID do host (variavel de ambiente ou `os-arch` detectado) e, se preciso, cai para um RID base. Em `hostpolicy_context.cpp` (`should_read_rid_fallback_graph`) o grafo so e lido se a configuracao `System.Runtime.Loader.UseRidGraph` for `true`; por padrao esta desligada.

### Probing: a lista ordenada

Do `host-probing.md`: para cada asset, o host percorre uma lista ordenada de diretorios, concatena `<library path>/<asset path>` e verifica se o arquivo existe. A ordem (prioridade decrescente):

1. **Servicing paths** (somente se `serviceable: true`): `%ProgramFiles(x86)%\coreservicing` no Windows x64 (`%ProgramFiles%\coreservicing` no x86; `$CORE_SERVICING` em Linux/Mac), com subpastas `|arch|` (so assets `runtime`) e `pkgs`.
2. **Diretorio do app** (ou do framework, ao resolver assets do framework).
3. **Diretorios de framework**, do mais alto para o mais baixo nivel (o app e o mais alto; para assets de um framework, so ele e os de nivel mais baixo).
4. **Shared store**: `$DOTNET_SHARED_STORE/|arch|/|tfm|` (varios caminhos possiveis) e `<dotnet.exe>/store/|arch|/|tfm|`.
5. **Additional probing paths**: `--additionalprobingpath`, `additionalProbingPaths` em `.runtimeconfig.json` e `.runtimeconfig.dev.json` (placeholders `|arch|` e `|tfm|` sao substituidos).

Se nada e achado, o host reporta erro, **exceto** para o `.deps.json` do proprio app, que e ignorado. Em apps *self-contained* nao ha dependencias de framework, entao tudo e procurado no diretorio do app.

Em modo single-file, `deps_resolver.cpp::resolve_tpa_list` marca o resultado como `probe_result_t::bundled` e **nao** adiciona esses assemblies a TPA: "o runtime sonda diretamente o manifesto do bundle via callback do host".

### Resolucao de conflitos app x framework

`docs/design/features/assembly-conflict-resolution.md` descreve o algoritmo (resumo fiel):

- Ordem dos `deps.json`: aplicacao, depois `--additional-deps`, depois frameworks do mais alto para o mais baixo.
- Para cada entrada: se ja existe um asset resolvido com o mesmo nome, compara **assembly version** e **file version**; se a nova e igual ou menor, **descarta**; se maior, substitui e faz o probing de novo.
- Para sondar, itera os probing paths exceto frameworks de nivel mais alto; se o path e um framework, so o usa se o `deps.json` dele contem o pacote exato (nome+versao).
- Se faltar metadado de versao (apps publicados antes da 2.1), o asset e considerado "mais velho" e nao substitui nada.
- Resultado final: lista de assemblies e caminhos passada ao CLR.

Efeito prático: "o app normalmente vence", porque entra primeiro na ordem e referencia pacotes OOB que o framework nao tem; mas um framework com versao maior de um mesmo assembly substitui.

### `deps_resolver_t`: o motor

`src/native/corehost/hostpolicy/deps_resolver.cpp`:

- `resolve_tpa_list(...)` (linha ~436): lambda `process_entry` com tres detalhes: ignora placeholders terminados em `/_._`; registra *breadcrumbs* para entradas serviceable; mantem um `name_to_resolved_asset_map_t` para que o primeiro nome vencedor (ou o de versao maior) ganhe.
- `resolve_probe_dirs(asset_type, ...)` produz as listas de diretorios `native` e `resources` (para `resources`, extrai a pasta-base removendo a cultura: `./foo/en-us/resource.dll` vira `./foo`).
- `resolve_probe_paths(...)` (linha ~898) chama as tres e atribui `coreclr` do `m_coreclr_path`.
- `--additional-deps`: so para framework-dependent; nao disponivel em self-contained (sem grafo RID/versao do framework) nem em cenarios libhost (comhost) (comentarios no proprio arquivo; ver tambem `docs/design/features/additional-deps.md`).

### O host fala com o runtime por propriedades e por um contrato

`src/native/corehost/hostpolicy/coreclr.cpp` (~linha 158) lista as chaves repassadas ao `coreclr_initialize`: `NATIVE_DLL_SEARCH_DIRECTORIES`, `PLATFORM_RESOURCE_ROOTS`, `APP_CONTEXT_BASE_DIRECTORY`, `APP_CONTEXT_DEPS_FILES`, `PROBING_DIRECTORIES`, `APP_PATHS`, alem da TPA. Mais recentemente existe tambem o **host runtime contract** (`src/native/corehost/host_runtime_contract.h`), uma struct de callbacks (`get_runtime_property`, `bundle_probe`, `pinvoke_override`, `external_assembly_probe`, `get_native_code_data`, `get_assembly_names`, `resolve_assembly_to_path`). Em `hostpolicy_context.cpp`, se o host nao definiu explicitamente `TRUSTED_PLATFORM_ASSEMBLIES`, `get_assembly_names`/`resolve_assembly_to_path` respondem a partir de tabelas internas (`tpa_names`, `tpa_paths`); caso contrario retornam `false` e o runtime usa a string da propriedade. Em `ApplicationContext::SetupBindingPaths` (`src/coreclr/binder/applicationcontext.cpp`) isso aparece como `HostInformation::GetAssemblyNames(...)` versus parse da string `sTrustedPlatformAssemblies`. Ou seja: a "TPA" pode ser entregue como **callback consultavel** em vez de uma string gigante.

### Resolucao para componentes (plugins)

`docs/design/features/host-component-dependencies-resolution.md` e `AssemblyDependencyResolver` reaproveitam o mesmo codigo do hostpolicy para um unico componente. O resolvedor gerenciado (`src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`) chama `corehost_resolve_component_dependencies` (`hostpolicy.cpp`, linha ~917), recebe tres listas (assemblies, caminhos nativos, caminhos de recursos) e as indexa. Frameworks **nao** sao processados nesse modo, de proposito (o plugin deve compartilhar o framework do app). Detalhes no arquivo 11.

```csharp
var resolver = new AssemblyDependencyResolver(@"C:\plugins\MyPlugin\MyPlugin.dll");
string? path = resolver.ResolveAssemblyToPath(new AssemblyName("Newtonsoft.Json"));
string? native = resolver.ResolveUnmanagedDllToPath("sqlite3");
```

Regra interna: a chave do dicionario e o nome simples case-insensitive (`StringComparer.OrdinalIgnoreCase`, justificado pelo lookup hash da TPA) e, havendo duplicatas, **a primeira entrada vence** (`TryAdd`).

### O que o runtime ainda faz depois do host

`BindByTpaList` (`src/coreclr/binder/assemblybindercommon.cpp`, ~linha 836) segue esta ordem para assembly neutro:

1. Probe extensions (single-file bundle / probe externo) **antes** da TPA.
2. Mapa TPA por nome simples (arquivo 04).
3. `APP_PATHS` (legado) via `BindAssemblyByProbingPaths`.
4. Falha => `ERROR_FILE_NOT_FOUND`.

Para assemblies com cultura nao neutra, entra `BindSatelliteResource` (arquivo 06).

### Pipeline ponta a ponta (resumo)

```
build/publish (SDK)                         startup (host)                         runtime
-------------------                         ---------------                        -------
app.runtimeconfig.json  ----------------->  hostfxr: escolhe framework (roll-fwd)
app.deps.json           ----------------->  hostpolicy: deps_resolver_t
Microsoft.NETCore.App.deps.json ---------->    - ordena app > additional-deps > fx
                                               - probing (servicing, app, fx, store, extra)
                                               - resolve_tpa_list / resolve_probe_dirs
                                         ---> coreclr_initialize(props, contract)
                                                                              ---> ApplicationContext::SetupBindingPaths
                                                                              ---> BindByTpaList (por nome)
```

### Propriedades produzidas e quem as consome

| Propriedade | Conteudo | Consumidor |
|---|---|---|
| `TRUSTED_PLATFORM_ASSEMBLIES` | caminhos de assemblies gerenciados | `ApplicationContext` (mapa TPA) |
| `NATIVE_DLL_SEARCH_DIRECTORIES` | diretorios de bibliotecas nativas | `LoadNativeLibraryBySearch` (arquivo 09) |
| `PLATFORM_RESOURCE_ROOTS` | raizes de satelites da plataforma | `BindSatelliteResource` (arquivo 06) |
| `APP_PATHS` | diretorios do app (legado) | `BindAssemblyByProbingPaths` |
| `APP_CONTEXT_BASE_DIRECTORY` / `APP_CONTEXT_DEPS_FILES` | base do app e deps.json em uso | `AppContext.BaseDirectory`, ferramentas |
| `PROBING_DIRECTORIES` | diretorios de probing efetivos | diagnostico |
| `BUNDLE_PROBE` (<= .NET 8) / `bundle_probe` do contrato | sondar o single-file | `AssemblyProbeExtension` |

### Tracing do host

`DOTNET_HOST_TRACE=1` imprime cada entrada processada (`Processing TPA for deps entry [...] with fx level: N`) e cada tentativa de probing; `DOTNET_HOST_TRACE_VERBOSITY` (1-4) controla o detalhe e `DOTNET_HOST_TRACEFILE` redireciona (`docs/design/features/host-tracing.md`). No lado do runtime, `BinderTracing` (`src/coreclr/binder/bindertracing.cpp`) emite eventos ETW/EventPipe como `AssemblyLoadStart` e registra a cadeia de tentativas (`PathProbed`, `PathSource::Bundle`, `PlatformResourceRoots`, `AppPaths`).

### Armadilhas comuns

- **Remover o `deps.json` do app**: o host passa a considerar os arquivos do diretorio do app, perdendo informacao de RID e de conflito de versao.
- **Pacotes com assets nativos por RID**: sem o RID correto (ou sem o grafo de fallback, que e opt-in via `System.Runtime.Loader.UseRidGraph`), os nativos nao sao encontrados.
- **Comparacao case-insensitive**: no Linux, nomes de arquivo diferem por caixa mas o mapa TPA trata nomes simples sem distinguir caixa.
- **`_._`**: arquivos placeholder em pacotes (`lib/net8.0/_._`) significam "compativel, sem assets" e nao devem falhar a resolucao.

## Por que

- **Reprodutibilidade e velocidade**: resolver caminhos em *build/publish* (o SDK escreve o `.deps.json`) e no startup do host evita que o runtime varra pastas em cada carga. O manifesto lista exatamente o que existe, o que torna o startup previsivel.
- **Separacao host/runtime**: o runtime nao conhece NuGet, RID, store nem servicing. Isso mantem o binder pequeno e permite hosts customizados (`docs/design/features/native-hosting.md`) preencherem as propriedades de outra maneira.
- **Servicing e shared store**: o design de 2016-2018 permitia corrigir pacotes por maquina (`coreservicing`) e compartilhar pacotes pre-compilados (`DotNetCore-SharedPackageStore.md`). Isso explica a ordem estranha (servicing primeiro).
- **Conflito por versao, nao por posicao**: quando app e framework trazem o mesmo assembly, vence o de maior versao, para que o app nao fique com um pacote mais velho que o framework espera.
- **Manifesto de asset por tipo (`runtime`/`native`/`resources`)** reflete as tres bases de busca diferentes: managed bind, `dlopen` e satelites.

## Trade-offs

- **Dois lugares com semantica de resolucao** (host + binder) -> bugs de fronteira, como as issues citadas em `native-hosting-load-context.md` (#127149, #118115, #95607, #66013, #104480).
- **Muitos probing paths historicos** (servicing, store, additional) que a maioria dos apps nunca usa, mas o formato e o codigo carregam essa herança.
- **Ordem implicita**: "o app normalmente vence" depende de o framework nao referenciar o mesmo pacote; nao e garantia estrutural.
- **Dependencia do SDK**: sem `deps.json`, comportamento degrada (o host usa os arquivos do diretorio do app); metadados de versao ausentes mudam o resultado do conflito.
- **TPA como string** nao escala bem (centenas de assemblies), o que motivou o contrato de callbacks.
- **Componentes sem framework**: `AssemblyDependencyResolver` nao enxerga frameworks nem `runtimeconfig`, e usa os mesmos probing paths do app (limitacao admitida nas "Open questions" do doc).

## Como aplicar na sua linguagem

1. **Introduza um manifesto de resolucao gerado em build** (ex.: `app.deps.toml`) com: id do modulo, versao, tipo de asset (codigo / nativo / recursos), caminho relativo, e RID/plataforma. O runtime nao deve percorrer diretorios para descobrir modulos.
2. **Faca o launcher (host) resolver, o runtime ligar.** Defina um contrato pequeno: "lista de (nome -> caminho)" + "diretorios nativos" + "raizes de recursos". Prefira *callbacks* a strings (como `get_assembly_names`/`resolve_assembly_to_path`) se espera milhares de modulos.
3. **Use uma lista de probing curta e fixa** (comece com: diretorio do app; diretorio da plataforma; caches configurados). Nao replique servicing/store a menos que tenha o problema real.
4. **Defina o conflito por versao semantica explicita**, nao por ordem: "vence a maior versao do modulo; empate vence o mais proximo do app". Grave a versao no manifesto (equivalente ao `assemblyVersion`/`fileVersion`).
5. **Trate o manifesto do proprio app como opcional** (como o host faz) para facilitar `run script.ext` sem build.
6. **Ofereca uma API de resolucao por componente** (`ComponentResolver(path)`) que reaproveite exatamente o codigo do launcher e *exclua a plataforma* por padrao, para plugins.
7. **Ignore placeholders** (`_._`) -- um marcador de "este pacote nao contribui com codigo" evita falhas por arquivo ausente.
8. **Exponha `--trace`** de resolucao (o host .NET tem `DOTNET_HOST_TRACE`; ver `docs/design/features/host-tracing.md`).

## Referencias

- `docs/design/features/host-probing.md`
- `docs/design/features/assembly-conflict-resolution.md`
- `docs/design/features/additional-deps.md`
- `docs/design/features/host-component-dependencies-resolution.md`
- `docs/design/features/host-components.md`, `host-runtime-information.md`, `host-tracing.md`, `native-hosting.md`
- `docs/design/features/sharedfx-lookup.md`, `framework-version-resolution.md`, `DotNetCore-SharedPackageStore.md`
- `src/native/corehost/hostpolicy/deps_resolver.cpp`, `deps_format.cpp`, `hostpolicy.cpp`, `hostpolicy_context.cpp`, `coreclr.cpp`
- `src/native/corehost/host_runtime_contract.h`
- `src/coreclr/binder/assemblybindercommon.cpp`, `applicationcontext.cpp`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/Loader/AssemblyDependencyResolver.cs`
- Nenhuma especificacao ECMA cobre `deps.json`; e um formato do ecossistema .NET Core (SDK + host).
