# Trusted Platform Assemblies (TPA)

> Objetivo: explicar o que e a TPA, como ela e construida e consumida, onde termina sua autoridade e como ela se relaciona com `APP_PATHS` e com o bind por nome.

## Mecanismo no .NET

### Definicao

A **TPA** (propriedade `TRUSTED_PLATFORM_ASSEMBLIES`, definida em `src/native/corehost/host_runtime_contract.h`) e uma lista de caminhos completos de assemblies que o host declara como "a plataforma + o grafo estatico do app". O contexto Default do runtime (`DefaultAssemblyBinder`) so liga *automaticamente* o que esta nessa lista (alem de recursos e `APP_PATHS`).

O "trusted" aqui nao e sandbox (o .NET Core abandonou CAS): e **confianca de origem**. O comentario de `BindByTpaList` (`src/coreclr/binder/assemblybindercommon.cpp`, ~linha 816) descreve o algoritmo como "o ponto de entrada do algoritmo de binding customizado do CoreCLR": primeiro recursos satelites; depois assemblies da TPA; depois, se houver, `APP_PATHS`.

### Estrutura em memoria

`ApplicationContext::SetupBindingPaths` (`src/coreclr/binder/applicationcontext.cpp`, linha ~82):

```
m_pTrustedPlatformAssemblyMap = new SimpleNameToFileNameMap();
// Duas fontes mutuamente exclusivas:
if (HostInformation::GetAssemblyNames(&names, &count))   // contrato do host (callbacks)
    para cada nome: se ja existe, continue; senao mapa[nome] = (arquivo ainda nao resolvido)
else
    percorre a string sTrustedPlatformAssemblies:
       GetNextTPAPath(..., dllOnly=false, fileName, simpleName)
       se simpleName ja existe -> continue   // PRIMEIRO VENCE
       mapa[simpleName] = fileName
```

Fatos verificaveis:

- Chave = **nome simples**, comparacao sem diferenciar maiusculas (o comentario em `AssemblyDependencyResolver.cs` fala em `SimpleNameToFileNameMapTraits`, "TPA lookup hash").
- **A primeira entrada vence**: duplicatas sao ignoradas (`LookupPtr != nullptr -> continue`). Portanto a *ordem* produzida pelo host e semantica.
- Na variante por callbacks, o mapa guarda apenas o nome; o arquivo e resolvido sob demanda via `resolve_assembly_to_path` (`host_runtime_contract.h`; implementado em `src/native/corehost/hostpolicy/hostpolicy_context.cpp`).

### Como o host produz a TPA

`deps_resolver_t::resolve_tpa_list` (`src/native/corehost/hostpolicy/deps_resolver.cpp`, ~linha 436) percorre as entradas de `deps.json` na ordem app -> `--additional-deps` -> frameworks (arquivo 03) e usa um mapa nome->asset:

```cpp
auto process_entry = [&](deps_dir, entry, fx_level) {
   if (breadcrumb && entry.is_serviceable) { ... }       // rastreio de servicing
   if (entry.asset.relative_path ends_with "/_._") return true;   // placeholder
   existing = items.find(entry.asset.name);
   if (existing == end) {
       probe_result = probe_deps_entry(entry, deps_dir, fx_level, &resolved_path);
       if (probe_result != not_found) {
           if (probe_result != bundled)               // single-file: nao entra na TPA
               add_tpa_asset(entry.asset.name, &entry.asset, resolved_path, &items);
           return true;
       }
   } else { /* compara versoes; substitui se maior */ }
};
```

`hostpolicy_context.cpp` guarda `tpa_names` e `tpa_paths` e, se algum host (ou o usuario via propriedade) ja definiu a TPA explicitamente, desliga os callbacks (`return false`) para que o runtime use essa string. `reconstruct_tpa_property` monta a string sob demanda (`get_runtime_property`) quando alguem a pede (por exemplo `AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")`).

### O passo de bind: TPA vs APP_PATHS

Trecho do algoritmo (comentarios em `BindByTpaList`):

1. Se *probe extensions* (single-file bundle, probe externo) estao habilitadas, procura `<nome>.dll` nelas **primeiro**: "bundled assemblies should only be in the bundle manifest, not in the TPA".
2. Procura o nome simples no mapa TPA. Se achou o arquivo e a referencia casa (`TestCandidateRefMatchesDef(..., tpaListAssembly=true)`), liga; se achou mas a identidade completa nao casa, **continua** para procurar em `APP_PATHS` (match parcial: `fPartialMatchOnTpa`).
3. Em `APP_PATHS`: se o assembly achado tambem existe na TPA e o *fullname sem versao* e igual, **liga a instancia da TPA** ("Fullname matched for TPA and app assembly - so bind to TPA instance"); se difere, liga a instancia do app.
   - Atencao: o comentario do codigo cita "SimpleName+Culture+PKT", mas `AssemblyName::Equals` (`assemblyname.cpp`) so compara o token de chave publica quando a flag `INCLUDE_PUBLIC_KEY_TOKEN` (0x10) e passada, e `TestCandidateRefMatchesDef` usa `INCLUDE_DEFAULT` (igual a 0x00, ver `inc/assemblyname.hpp`) mais `INCLUDE_ARCHITECTURE` opcional.
   - Na pratica a comparacao e por **nome simples + cultura** (+ arquitetura, se a referencia a especifica).
4. Nao achou: `ERROR_FILE_NOT_FOUND`; o chamador (`BindUsingAssemblyName`) tenta o ALC gerenciado.

`APP_PATHS` e um mecanismo legado: `hostpolicy_context.cpp` (~linha 384) so o preenche com o switch de compatibilidade; `AssemblyDependencyResolver.cs` diz que "essa feature esta sendo efetivamente descontinuada".

### Quem pode estar fora da TPA

- Qualquer coisa carregada por `LoadFromAssemblyPath/Stream` em contexto custom nunca entra na TPA.
- `Assembly.LoadFrom` carrega no Default **por caminho** (nao precisa estar na TPA).
- Em `BindUsingPEImage` (binder comum), um assembly carregado por caminho no Default e comparado com a TPA: se o nome e a identidade casam com um da TPA, liga o da TPA (`OtherBindInterfered`, `RegisterAndGetHostChosen`) -- o host "escolhe".

### CoreLib e TPA

`System.Private.CoreLib` nao passa por `BindByTpaList`: `AssemblyBinderCommon::BindToSystem` o carrega do `SYSTEM_CORELIB_DIRECTORY` ou ao lado do `coreclr` (`host-runtime-information.md`), e `BindToSystemSatellite` cuida dos seus recursos. Os binders marcam `fIsInTPA = TRUE` ao criar esses `Assembly` (ver `assemblybindercommon.cpp`, linhas ~305-381).

### Exemplo: inspecionar a TPA em tempo de execucao

```csharp
var tpa = (string)AppContext.GetData("TRUSTED_PLATFORM_ASSEMBLIES")!;
foreach (var p in tpa.Split(Path.PathSeparator))
    Console.WriteLine(Path.GetFileNameWithoutExtension(p) + " -> " + p);
```

E um host nativo minimo define a propriedade manualmente (ver `docs/design/features/native-hosting.md`):

```c
const char* keys[]   = { "TRUSTED_PLATFORM_ASSEMBLIES", "APP_PATHS", "NATIVE_DLL_SEARCH_DIRECTORIES" };
const char* values[] = { "/rt/System.Private.CoreLib.dll:/rt/System.Runtime.dll:/app/app.dll",
                         "/app", "/app/natives" };
coreclr_initialize(exePath, "host", 3, keys, values, &hostHandle, &domainId);
```

### Fluxograma de `BindByTpaList` (neutro)

```
pedido: nome simples N, cultura neutra
 |
 +-- probe extension habilitada? --> procura N.dll no bundle / probe externo --> achou: liga e sai
 |
 +-- N esta no mapa TPA? 
 |      sim: GetAssembly(caminho, fIsInTPA=TRUE)
 |            referencia casa com a definicao (TestCandidateRefMatchesDef)? --> liga e sai
 |            nao casa: marca fPartialMatchOnTpa e continua
 |      nao: continua
 |
 +-- APP_PATHS (BindAssemblyByProbingPaths) e excludeAppPaths == false?
 |      achou em app paths:
 |         tambem estava na TPA e mesmo fullname --> liga a instancia da TPA
 |         nome diferente                         --> liga a instancia do app
 |         nunca visto na TPA                     --> liga a instancia do app
 |
 +-- nada: retorna S_FALSE -> BindByName interpreta como ERROR_FILE_NOT_FOUND
```

Depois do bind, `BindLocked` aplica `IsCompatibleAssemblyVersion(pedido, achado)`: se o achado for **mais velho** que o pedido, o resultado e `FUSION_E_APP_DOMAIN_LOCKED` e o bind falha (arquivo 05). O resultado e registrado no contexto de execucao por `RegisterAndGetHostChosen` (`BindAssembly` em `assemblybindercommon.cpp`), portanto uma segunda tentativa encontra o assembly via `FindInExecutionContext` sem voltar ao mapa. Esse registro usa **controle otimista de concorrencia**: o bind ocorre sob lock do `ApplicationContext`, guarda `kContextVersion`, e se outro bind "interferiu" (`OtherBindInterfered`), `RegisterAndGetHostChosen` devolve `S_FALSE` e todo o bind e refeito (`goto Retry`).

### Efeitos observaveis para o desenvolvedor

| Situacao | Resultado |
|---|---|
| `Assembly.Load("System.Linq")` | Resolve pelo mapa TPA, sem varrer o disco |
| Copiar `Foo.dll` para a pasta do app sem listar no `deps.json` | Nao esta na TPA; so achado se `APP_PATHS` estiver populado (opt-in) |
| Duas versoes de `Foo` na TPA | Impossivel: primeira entrada vence |
| `Foo.dll` v2 pedida, v1 na TPA | `FileLoadException` por `FUSION_E_APP_DOMAIN_LOCKED` (no Default o codigo e re-mapeado para `REF_DEF_MISMATCH`) |
| Plugin num ALC custom pede `System.Runtime` | Passo 3: resolve no Default (identidade compartilhada) |

## Por que

- **Eliminar varredura de diretorios**: com um mapa nome->arquivo, o bind e O(1). O .NET Framework tinha GAC + probing heuristico (`bin`, subpastas por cultura), caro e imprevisivel.
- **Plataforma e app como unidade**: o host sabe quais assemblies compoem "este app em esta maquina" (apos roll-forward de framework). Passar isso numa lista pronta tira conhecimento de layout do runtime.
- **Primeiro vence** simplifica semantica: o host decide o desempate ao ordenar; o runtime nao precisa entender versoes de pacote.
- **Prioridade da TPA sobre `APP_PATHS`** mantem o contrato "plataforma estavel"; so quando nome/cultura diferem na comparacao de `TestCandidateRefMatchesDef` o app pode introduzir um assembly com o mesmo nome simples.
- **Bundle antes da TPA** garante que o single-file nao dependa de arquivos soltos.

## Trade-offs

- **Nome simples como chave** significa que uma segunda versao do mesmo assembly no Default e impossivel: so mediante ALC diferente (o doc: "Multiple assemblies with the same simple name cannot be loaded into a single load context").
- **Primeiro vence** e fragil: a ordem do host (e portanto do `deps.json`) torna-se parte da API.
- **Strong-name quase decorativo**: o doc diz que o .NET Core "ignora o strong name token no processo de binding", e o codigo confirma: `TestCandidateRefMatchesDef` compara nome e cultura (e arquitetura, se pedida), sem PKT nem versao. Veja arquivo 05.
- **String gigante** de caminhos: custo de parse, de memoria; o contrato de callbacks reduz mas convive com a forma antiga.
- "Trusted" pode ser mal entendido como seguranca; nao e fronteira de seguranca.
- Ordem de precedencia TPA x `APP_PATHS` x bundle e sutil e depende de flags (`excludeAppPaths`, `skipVersionCompatibilityCheck`).

## Como aplicar na sua linguagem

1. **Tenha um "mapa de plataforma" por nome**: `nome_do_modulo -> caminho` produzido pelo launcher, imutavel apos o startup. Chave canonica (minusculas) e **primeira-ocorrencia-vence** com a ordem de insercao documentada.
2. **Entregue o mapa por interface consultavel** (`resolve(name) -> path`) em vez de string, e deixe o runtime guardar so os nomes ate o primeiro uso (como o `ApplicationContext` faz).
3. **Separe "modulos confiaveis da plataforma" de "modulos do app"** logicamente, mesmo que estejam no mesmo mapa; registre a origem em cada entrada para diagnostico (`fIsInTPA`).
4. **Nao confunda confianca com seguranca**: se precisar de sandbox, projete-o separadamente (capabilities, processos).
5. **Defina quem pode quebrar o mapa**: no .NET, so contextos customizados e `APP_PATHS`. Documente a mesma regra (por exemplo, "um modulo local so substitui um da plataforma se for declarado em `override`").
6. **Trate bundle/embedded como camada acima do mapa**: se o binario unico contem o modulo, nunca consulte o disco.
7. **Exponha a lista** (`runtime.platform_modules()`), pois ferramentas (linkers, IDE, depuradores) precisam dela e o .NET a expoe via `AppContext`.
8. **Ofereca modo "TPA explicita"** para hosts embarcados: o host define o mapa e desliga o resolvedor padrao (padrao em `hostpolicy_context.cpp`).

## Referencias

- `src/native/corehost/host_runtime_contract.h` (propriedades e callbacks)
- `src/native/corehost/hostpolicy/deps_resolver.cpp` (`resolve_tpa_list`, `add_tpa_asset`)
- `src/native/corehost/hostpolicy/hostpolicy_context.cpp` (`get_assembly_names`, `resolve_assembly_to_path`, `reconstruct_tpa_property`)
- `src/coreclr/binder/applicationcontext.cpp` (`SetupBindingPaths`)
- `src/coreclr/binder/assemblybindercommon.cpp` (`BindByTpaList`, `TestCandidateRefMatchesDef`, `BindToSystem`)
- `docs/design/features/host-runtime-information.md`, `native-hosting.md`, `assembly-conflict-resolution.md`
- ECMA-335 Partition II 6.3 (referencias a assembly): a TPA e uma implementacao; a CLI nao exige lista previa.
