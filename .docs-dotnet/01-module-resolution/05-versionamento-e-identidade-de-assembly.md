# Versionamento e identidade de assembly (nome, versao, cultura, public key token)

> Objetivo: mostrar o que o binder do CoreCLR realmente compara quando decide que "o assembly pedido e o assembly achado", e como isso difere do modelo historico do .NET Framework (strong naming + GAC).

## Mecanismo no .NET

### A identidade textual

Um assembly e referenciado por um *display name*:

```
Newtonsoft.Json, Version=13.0.0.0, Culture=neutral, PublicKeyToken=30ad4fe6b2a6aeed
```

No codigo nativo, `BINDER_SPACE::AssemblyIdentity` / `AssemblyName` (`src/coreclr/binder/inc/assemblyidentity.hpp`, `assemblyname.hpp`) mantem flags de quais partes existem (`IDENTITY_FLAG_SIMPLE_NAME=0x001`, `VERSION=0x002`, `PUBLIC_KEY_TOKEN=0x004`, `PUBLIC_KEY=0x008`, `CULTURE=0x010`, `PROCESSOR_ARCHITECTURE=0x040`, `RETARGETABLE=0x080`, `PUBLIC_KEY_TOKEN_NULL=0x100`). A conversao texto <-> estrutura vive em `textualidentityparser.cpp` (inclui `, Retargetable=Yes`). Os metadados vem do PE: `AssemblyName::Init(PEImage)` (`assemblyname.cpp`, ~linhas 50-145) le nome, versao, cultura, flags e (token de) chave publica da tabela `Assembly`/`AssemblyRef`. Duas validacoes ali: so o `ContentType` default e aceito (`IsAfContentType_Default`), e `IsAfRetargetable` liga o flag retargetable.

### O que `Equals` compara de fato

`AssemblyName::Equals(pOther, dwIncludeFlags)` (`assemblyname.cpp`, linha ~293):

```cpp
if (simpleName.EqualsCaseInsensitive(other.simpleName)) {
    equal = true;
    if (!(flags & EXCLUDE_CULTURE))            equal = normalizedCulture == other.normalizedCulture (case-insensitive);
    if (equal && (flags & INCLUDE_PUBLIC_KEY_TOKEN)) equal = pkt == other.pkt;
    if (equal && (flags & INCLUDE_ARCHITECTURE))     equal = arch == other.arch;
    if (equal && (flags & INCLUDE_VERSION))          equal = version.Equals(other.version);
    if (equal && (flags & INCLUDE_RETARGETABLE))     equal = retargetable == other.retargetable;
}
```

Com os valores de `inc/assemblyname.hpp`: `INCLUDE_DEFAULT=0x00`, `INCLUDE_VERSION=0x01`, `INCLUDE_ARCHITECTURE=0x02`, `INCLUDE_RETARGETABLE=0x04`, `INCLUDE_PUBLIC_KEY_TOKEN=0x10`, `EXCLUDE_CULTURE=0x20`. A cultura vazia e normalizada para `"neutral"` (`GetNormalizedCulture`).

A funcao que decide "a referencia casa com a definicao" e `TestCandidateRefMatchesDef` (`assemblybindercommon.cpp`, linha ~578): usa `INCLUDE_DEFAULT`, acrescenta `EXCLUDE_CULTURE` se (nao TPA e referencia neutra) e `INCLUDE_ARCHITECTURE` se a referencia especifica arquitetura. Resultado: **identidade efetiva do bind = nome simples (case-insensitive) + cultura (+ arquitetura quando pedida)**. Versao e PKT *nao* participam da igualdade nesse teste; o docs/design diz explicitamente: "Multiple assemblies with the same simple name cannot be loaded into a single load context ... Also, .NET Core ignores strong name token for assembly binding process" (`docs/design/features/assemblyloadcontext.md`).

### Versao: compatibilidade unidirecional

A versao e comparada separadamente por `IsCompatibleAssemblyVersion` (`assemblybindercommon.cpp`, linha ~51), documentada no codigo como "the assembly equivalence relation":

- Componentes nao especificados na referencia (`!HasMajor()`, `!HasMinor()`, ...) casam com qualquer valor.
- Compara major, minor, build, revision em ordem lexicografica.
- Se a versao **pedida e maior** que a encontrada em algum componente (e os de ordem maior sao iguais) -> **incompativel**.
- Se a pedida e **menor** -> compativel, e para de olhar componentes de menor ordem.
- Se a encontrada nao tem o componente que a pedida especificou -> incompativel.

Logo: **pedir 5.0.0.0 e achar 6.0.0.0 funciona; pedir 6.0.0.0 e achar 5.0.0.0 falha.** Isso e a politica que permite o framework "andar para frente" sem recompilar apps.

A verificacao ocorre em `BindLocked` (linha ~472): tanto quando o assembly ja esta no contexto de execucao (`FindInExecutionContext`) quanto depois de `BindByTpaList`. Em falha devolve `FUSION_E_APP_DOMAIN_LOCKED`; o `DefaultAssemblyBinder` traduz isso para `FUSION_E_REF_DEF_MISMATCH` (`defaultassemblybinder.cpp`). Existe um modo `skipVersionCompatibilityCheck` (usado para cargas por caminho/bytes, onde o chamador ja decidiu).

### Versao de assembly vs versao de arquivo

O host usa duas versoes diferentes, ambas gravadas no `.deps.json`: `assemblyVersion` e `fileVersion` (`src/native/corehost/hostpolicy/deps_format.cpp` registra-as por entrada, e `deps_resolver.cpp` as compara). `docs/design/features/assembly-conflict-resolution.md`: "check the assembly and file version of the new entry against the already resolved one. If the new entry is equal or lower, skip it". Ou seja, a politica *do host* desempata por versao (assembly e depois arquivo) antes de o runtime ver o nome; a politica *do runtime* so garante que a versao achada nao e menor que a pedida.

### Cache de falhas

`AssemblyBinderCommon::BindByName` consulta primeiro `pApplicationContext->GetFailureCache()` com o display name **com versao** (`AssemblyName::INCLUDE_VERSION`). Falhas nao transitorias (tudo menos `FILE_NOT_FOUND` quando `skipFailureCaching`) sao cacheadas: o mesmo pedido falho nao repete o trabalho.

### Cultura: o eixo dos satelites

`IsNeutralCulture()` separa dois caminhos em `BindByTpaList`: cultura neutra -> mapa TPA; cultura nao neutra -> `BindSatelliteResource` (arquivo 06). Em `TestCandidateRefMatchesDef`, para assemblies fora da TPA com referencia neutra, a cultura e excluida da comparacao (`EXCLUDE_CULTURE`).

### Arquitetura (processor architecture)

`IsValidArchitecture` (`assemblybindercommon.cpp`) rejeita referencias com arquitetura incompativel antes do bind (`FUSION_E_INVALID_NAME`) e, no bind por PE image, `CLR_E_BIND_ARCHITECTURE_MISMATCH`. `TranslatePEToArchitectureType` mapeia os flags do PE (`PEKIND`) para a identidade.

### Retargetable e ContentType

O flag `Retargetable` (`IDENTITY_FLAG_RETARGETABLE`) existe na identidade, mas o unico `ContentType` aceito e o default (o codigo rejeita qualquer outro, ver `assemblyname.cpp` linha ~116). Para uma linguagem nova: nao herde esses conceitos legados.

### Superficie gerenciada

`System.Reflection.AssemblyName` (em `src/libraries/System.Private.CoreLib/src/System/Reflection/AssemblyName.cs`) expoe `Name`, `Version`, `CultureName`, `GetPublicKeyToken()`, `ProcessorArchitecture`; `AssemblyLoadContext.GetAssemblyName(path)` (linha ~305 de `AssemblyLoadContext.cs`) le a identidade de um arquivo **sem carrega-lo**. `AssemblyDependencyResolver.ResolveAssemblyToPath` usa apenas `Name` e `CultureName` (tabela por nome simples), consistente com o binder.

```csharp
AssemblyName n = AssemblyLoadContext.GetAssemblyName(@"C:\libs\Lib.dll"); // sem carregar
Console.WriteLine($"{n.Name} {n.Version} [{n.CultureName}] pkt={Convert.ToHexString(n.GetPublicKeyToken() ?? [])}");
```

### Tabela de decisao: versao pedida x versao achada

| Pedida | Achada | `IsCompatibleAssemblyVersion` | Motivo |
|---|---|---|---|
| `5.0.0.0` | `5.0.0.0` | compativel | iguais |
| `5.0.0.0` | `6.0.0.0` | compativel | major pedido < achado: retorna `true` de imediato |
| `6.0.0.0` | `5.0.0.0` | incompativel | major pedido > achado |
| `6.1.0.0` | `6.0.9.9` | incompativel | major igual, minor pedido > achado |
| `6.0.0.0` | `6.0.0.0` | compativel | todos os componentes iguais |
| `6` (so major) | `6.3.2.1` | compativel | minor/build/revision nao especificados casam com qualquer valor |
| `6.0.0.5` | `6.0.0` (sem revision) | incompativel | achado sem o componente especificado |

(Os casos seguem linha a linha o corpo de `IsCompatibleAssemblyVersion`; o ultimo caso corresponde ao teste `!pFoundVersion->HasRevision()`.)

### Onde cada parte da identidade e consumida

| Parte | Binder (`src/coreclr/binder`) | Host (`src/native/corehost`) | Gerenciado |
|---|---|---|---|
| Nome simples | chave do mapa TPA, `Equals` | chave de `name_to_resolved_asset_map_t` | chave de `_assemblyPaths` (`OrdinalIgnoreCase`) |
| Cultura | ramo satelite vs neutro | tipo de asset `resources` | `CultureName` em `ResolveAssemblyToPath` |
| Versao | `IsCompatibleAssemblyVersion`, chave do cache de falhas | `assemblyVersion`/`fileVersion` para conflito | `AssemblyName.Version` |
| PKT | so se `INCLUDE_PUBLIC_KEY_TOKEN` for passado (nao e, no bind padrao) | nao usado | `GetPublicKeyToken()` informativo |
| Arquitetura | `IsValidArchitecture`, flag opcional em `Equals` | nao | `ProcessorArchitecture` (obsoleto para decisoes) |

### Exemplo: reproduzindo a politica de versao em C#

```csharp
// Espelho simplificado de IsCompatibleAssemblyVersion (componentes sempre presentes).
static bool IsCompatible(Version requested, Version found)
{
    if (requested.Major != found.Major)   return requested.Major < found.Major;
    if (requested.Minor != found.Minor)   return requested.Minor < found.Minor;
    if (requested.Build != found.Build)   return requested.Build < found.Build;
    return requested.Revision <= found.Revision;
}
```

Note a diferenca: o codigo nativo trata componentes ausentes (`-1` no `System.Version`) como curingas na referencia e como falha no encontrado; a versao acima nao modela isso.

### Evolucao historica (por que o modelo atual e simples)

- No .NET Framework, a identidade completa (incluindo PKT) era parte do bind, havia GAC, `bindingRedirect` e verificacao de strong name; `docs/design/features/assemblyloadcontext.md` cita o post de Suzanne Cook sobre os "binding contexts" desse mundo.
- No .NET Core, o `Default` passou a ser uma TPA preenchida pelo host, e o ALC tornou explicito o que antes eram "contextos" implicitos. A identidade ficou reduzida ao necessario para manter o mapa por nome funcionando.
- `System.Private.CoreLib` e a nomenclatura de facades (`System.Runtime`) dependem de *type forwarding* (metadados `ExportedType`), nao de binding por PKT; a referencia continua pedindo o nome da facade e o metadado leva ao assembly de implementacao.

## Por que

- **Strong name nunca foi seguranca**: a chave publica sem verificacao de assinatura em runtime so garante *unicidade de nome*. O .NET Core removeu a verificacao e o GAC; manter o token na comparacao causaria `FileLoadException` por refs com tokens divergentes (por exemplo, a migracao `mscorlib` -> `System.Private.CoreLib` com type forwarding).
- **Compatibilidade "para frente"**: aceitar `found >= requested` permite que um framework mais novo satisfaca apps compilados contra versoes antigas, sem arquivos de redirecionamento de binding (`bindingRedirect` do .NET Framework).
- **Nome simples + cultura como chave**: bate com a estrutura em disco (`<cultura>/<nome>.resources.dll`) e com o mapa TPA, mantendo o binder O(1).
- **Duas versoes (assembly e arquivo)**: `AssemblyVersion` e contrato de compatibilidade binaria (muda raramente); `FileVersion` rastreia builds/servicing. Separar permite corrigir bugs (patch) sem quebrar referencias.
- **Cache de falhas**: a resolucao pode ser cara (probing + callbacks gerenciados); falhas determinísticas sao memorizadas.

## Trade-offs

- **Colisoes silenciosas**: dois assemblies diferentes com o mesmo nome simples e cultura se confundem; apenas ALCs diferentes escapam.
- **Sem redirecionamento explicito**: o usuario nao pode dizer "para esta ref, use esta versao" a nao ser via ALC (`Load`) ou `Resolving`.
- **Compatibilidade unidirecional nao detecta quebra semantica**: `found >= requested` nao garante que a API continue igual.
- **PKT preservado nos metadados mas ignorado**: ferramentas (ex.: assinatura, auditoria) ainda os usam; a divergencia causa confusao ("por que o token nao importa?").
- **Cultura como parte da identidade** adiciona um ramo inteiro (satelites) a um binder que seria muito mais simples sem localizacao embutida.
- **Display names sao frageis** (escaping, `Retargetable`, `ProcessorArchitecture`); `TextualIdentityParser` e codigo nao trivial.

## Como aplicar na sua linguagem

1. **Defina identidade minima**: `(nome, versao)` com versao semantica (`major.minor.patch`), mais `cultura` apenas se localizacao for parte do modelo de modulo. Evite PKT; se precisar de integridade, use **hash de conteudo** num lockfile (separado da identidade).
2. **Escreva a relacao de compatibilidade como funcao pura e testada**, como `IsCompatibleAssemblyVersion`: `compatible(requested, found)`. Sugestao: mesmo major e `found >= requested` (semver), ou documente seu proprio contrato. Teste com tabela de casos (incluindo componentes ausentes).
3. **Separe "versao de contrato" de "versao de build"** (analogo a assembly/file version) e grave ambas no manifesto para desempates de conflito.
4. **Compare nomes sem diferenciar caixa apenas se o sistema de arquivos alvo exigir**; o .NET usa comparacao case-insensitive no mapa TPA mas arquivos em Linux sao case-sensitive -- evite essa assimetria definindo nomes de modulo canonicos em minusculas.
5. **Ofereca `read_identity(path)` sem carregar** (como `GetAssemblyName`) -- essencial para ferramentas e plugin discovery.
6. **Cache de falhas** por (contexto, nome, versao) com invalidacao explicita; registre o motivo (diagnostico) junto da falha.
7. **Se quiser redirecionamento**, torne-o explicito e auditavel (uma tabela `redirect` no manifesto) em vez de espalhado em hooks.
8. **Valide arquitetura/plataforma antes do bind** e retorne um erro distinto de "nao achei" (como `CLR_E_BIND_ARCHITECTURE_MISMATCH`).

## Referencias

- `src/coreclr/binder/assemblyname.cpp` (`Init`, `Equals`, `GetDisplayName`, `GetNormalizedCulture`)
- `src/coreclr/binder/inc/assemblyidentity.hpp`, `inc/assemblyname.hpp`, `inc/assemblyversion.hpp`
- `src/coreclr/binder/textualidentityparser.cpp`
- `src/coreclr/binder/assemblybindercommon.cpp` (`IsCompatibleAssemblyVersion`, `TestCandidateRefMatchesDef`, `BindLocked`, `BindByName`)
- `src/coreclr/binder/failurecache.cpp`
- `src/native/corehost/hostpolicy/deps_format.cpp`, `deps_resolver.cpp`
- `docs/design/features/assemblyloadcontext.md`, `assembly-conflict-resolution.md`
- ECMA-335 Partition II secao 6.2 (Assembly: Name, Version, Culture, PublicKey), secao 22.2 (tabela Assembly) e 22.5 (AssemblyRef), 23.1.2 (AssemblyFlags). Nota: o mecanismo de resolucao em tempo de execucao e "implementation-defined" pela ECMA-335; o .NET Core o simplificou em relacao ao Framework.
