# Strong naming e assinatura de assembly

## Mecanismo no .NET

"Strong name" é um mecanismo de **identidade** de assembly baseado em criptografia de chave pública. Um assembly com strong name tem identidade completa:

```
(nome simples, versão, cultura, public key token)
```

e carrega uma **assinatura digital** (RSA sobre um hash do conteúdo) numa região reservada do arquivo. A doc do repo (`docs/project/strong-name-signing.md`) é direta sobre o propósito atual: strong names são "left over from previous eras of .NET where sandboxing needed to differentiate between code that was trusted, versus code that was untrusted"; hoje servem para identidade, não para segurança ("strong-names are used for identity, and not for security").

### Onde aparece no formato

1. **Header CLI** (`IMAGE_COR20_HEADER`, `src/coreclr/inc/corhdr.h`): `IMAGE_DATA_DIRECTORY StrongNameSignature` (região reservada para a assinatura) e flag `COMIMAGE_FLAGS_STRONGNAMESIGNED = 0x00000008`.
2. **Tabela `Assembly`** (token `mdtAssembly 0x20000000`): contém `PublicKey` (blob com a **chave pública completa**), `Name`, `Culture`, `Version` (4 componentes), `Flags`. Flag `afPublicKey = 0x0001` ("The assembly ref holds the full (unhashed) public key").
3. **Tabela `AssemblyRef`** (`mdtAssemblyRef 0x23000000`): referencia outro assembly por nome + versão + cultura + **PublicKeyOrToken** (normalmente o **token** de 8 bytes; se `afPublicKey` setada, a chave completa).
4. `CorAssemblyFlags` ainda traz: `afRetargetable = 0x0100`, `afPA_*` (arquitetura de processador: MSIL, x86, IA64, AMD64, ARM, ARM64, `afPA_NoPlatform` para reference assemblies), `afContentType_*` (Default/WindowsRuntime), e `afEnableJITcompileTracking`/`afDisableJITcompileOptimizer` ("From DebuggableAttribute").

### Public key token

`src/coreclr/md/runtime/strongnameinternal.cpp`, `StrongNameTokenFromPublicKey`: verifica que a chave é válida (`StrongNameIsValidPublicKey`); consulta um cache de chaves bem-conhecidas; senão calcula **SHA-1 do blob da chave pública** e pega os **últimos 8 bytes** do hash, invertendo a ordem ("These are the low order bytes from a network byte order point of view. Reverse the order of these bytes in the output buffer to get host byte order"). `StrongNameToken::SIZEOF_TOKEN = 8` (`src/coreclr/inc/strongnameinternal.h`). O formato do blob de chave é `PublicKeyBlob { SigAlgID, HashAlgID, cbPublicKey, PublicKey[] }` (CryptoAPI).

O runtime também embute a chave da biblioteca base: `g_coreLibPublicKey`/`g_coreLibPublicKeyLen` (`strongnameinternal.cpp`, usado em `src/coreclr/vm/baseassemblyspec.cpp` para reconhecer *satellite assemblies* de `System.Private.CoreLib.resources`).

### Uso na ligação (binder)

`src/coreclr/binder/assemblyname.cpp` — `AssemblyName::Equals(pAssemblyName, dwIncludeFlags)`: compara nome simples (case-insensitive) e, conforme flags de inclusão, cultura, **public key token** (`INCLUDE_PUBLIC_KEY_TOKEN`), arquitetura, versão e *retargetable*. Quando o `AssemblyRef` traz `afPublicKey`, o binder converte a chave em token via `GetTokenFromPublicKey` (`binder/utils.cpp`, que usa `StrongNameTokenFromPublicKey`). Ou seja, em .NET moderno a "identidade de strong name" é **nome + token** e a política de versão é permissiva; o runtime **não valida a assinatura criptográfica**: em `src/coreclr/inc/pedecoder.inl`, `PEDecoder::CheckStrongNameSignature()` só valida o **diretório** (`CheckDirectory(&GetCorHeader()->StrongNameSignature, IMAGE_SCN_MEM_WRITE, NULL_OK)`), isto é, se a região existe e é bem formada. A doc de public signing confirma: *"public signing ... allows you to load the assembly in most contexts, or more precisely in any context that doesn't require validating the strong-name signature"*.

### Public signing (assinatura pública)

`docs/project/public-signing.md`: todo assembly do .NET Core é strong-named; para permitir que qualquer um construa assemblies com a **mesma identidade** sem a chave privada, usa-se **public signing**: o compilador (`/publicsign`, MSBuild `<PublicSign>True</PublicSign>`, com `<DelaySign>False</DelaySign>`) coloca a chave pública no manifesto e **liga o bit** `STRONGNAMESIGNED` (0x8) sem calcular a assinatura (como *delay signing*, mas sem precisar registrar "skip verification" na máquina). Dá para ver com `corflags.exe`: `CorFlags: 0x2000b` para strong-named/public-signed (bit 0x8 ligado); `sn.exe -vf` reporta `delay-signed or test-signed assembly` ou `Failed to verify assembly -- Strong name validation failed` para um binário public-signed. Existia também a ferramenta `FakeSign` (NuGet) para ligar/desligar o bit. Limitações no .NET Framework (GAC, shadow copy, parcial trust, NGen, pré-compilação do ASP.NET); no .NET Core, o uso de assemblies public-signed em produção é suportado, embora a API do assembly privado "is unsupported".

### Por que o .NET Core mantém strong names

`docs/project/strong-name-signing.md`: (1) **compatibilidade** — manter identidade de tipo com versões anteriores; remover o strong name é *breaking change*; (2) **serviceability** — no .NET Framework alguns assemblies do .NET Core são distribuídos "app-local", e a manutenção por patch de segurança exige strong name.

A mesma doc lista os 3 problemas de aplicar strong names: **política de binding** estrita no Framework (redirects), **viralidade** (um assembly strong-named só pode referenciar estaticamente outros strong-named) e **não ser drop-in replacement** quando a chave privada não é pública — resolvido com public signing. E recomenda que **bibliotecas públicas** usem strong name (para não excluir consumidores que precisam dele), mas aplicações em geral não precisam.

### Interação com R2R/Composite

Imagens R2R são assemblies com código nativo anexado (arquivo 05). O crossgen2 oferece `--compositekeyfile` (`Crossgen2RootCommand.cs`): para um build **composite**, aceita um arquivo com **chave pública** (validada por `IsValidPublicKey` em `Program.cs`, que também aceita a "ECMA key") que é gravada em `CompositeImageSettings.PublicKey` — dando uma identidade de strong name ao assembly composto gerado. Mensagem de erro: `ErrorCompositeKeyFileNotPublicKey` quando o arquivo não é uma chave pública.

Em Native AOT, strong names perdem o sentido operacional: o executável é uma única imagem nativa sem binding dinâmico de assemblies; a identidade é só informativa (metadata de reflexão).

### Assinatura "de código" ≠ strong name

Strong naming é independente de **Authenticode** (assinatura do PE pelo SO) e de assinatura de pacotes NuGet; a aplicação de assinatura de código fica no pipeline de build/release.

## Por quê

- **Identidade global e anti-colisão**: dois autores diferentes podem ter um `Utils.dll` sem conflito se as chaves diferem. A identidade completa permite o GAC (side-by-side) no .NET Framework.
- **Integridade (originalmente)**: a assinatura permitia detectar adulteração; o hash cobre o conteúdo exceto a região da assinatura.
- **Compatibilidade binária**: o token no `AssemblyRef` casa referência e definição de forma estável entre versões.
- **Tamanho**: o token de 8 bytes é muito menor que o blob da chave pública completa.

## Trade-offs

| Decisão | Benefício | Custo |
|---|---|---|
| Identidade = nome + versão + cultura + token | Desambigua | Políticas de versão rígidas e viralidade |
| Token de 8 bytes (últimos 8 bytes do SHA-1) | Compacto | Não é resistente a colisões por padrão moderno (SHA-1 truncado) — por isso é só identidade, não segurança |
| Assinatura RSA no arquivo | Detecta alteração (se validada) | Requer gerenciar chaves; o runtime moderno nem valida a assinatura |
| Chave privada guardada pelo autor | Controle de identidade | Quem constrói forks não consegue produzir identidade idêntica → public signing |
| Strong name obrigatório em libs | Compatibilidade ampla | Atrito para todos; pouco benefício de segurança |
| Remover strong name | Simplicidade | *Breaking change* para consumidores existentes |

## Como aplicar na sua linguagem

1. **Separe identidade de segurança.** Decida se o seu "assembly/módulo" precisa de uma identidade criptográfica ou basta nome + versão. Se precisar, use **hash de chave pública truncado** como identificador compacto (o .NET usa 8 bytes), mas **não** o venda como garantia de segurança.
2. **Guarde a chave completa no módulo e o token na referência.** Referências usam o token; o módulo definidor contém a chave. Habilite a conversão (chave → token) numa função única, com cache para chaves conhecidas, como `StrongNameTokenFromPublicKey`.
3. **Reserve uma região de assinatura** no cabeçalho do arquivo e um **bit de estado** ("assinado") para permitir **public signing/delay signing**: builds de terceiros mantêm a identidade sem possuir a chave privada.
4. **Defina a política de versão cedo**: o .NET Framework exigia match exato (dor: binding redirects); o .NET Core relaxou (versão maior satisfaz). Considere o modelo "versão >= requerida".
5. **Evite viralidade**: não exija que dependências de um módulo assinado também sejam assinadas, a menos que seja essencial.
6. **Faça a validação criptográfica explícita e separada do carregamento** (ex.: verificação em instalação/distribuição), em vez de custo em cada load. O CLR moderno não valida em tempo de carga.
7. **Para imagens pré-compiladas compostas**, dê uma identidade explícita (como `--compositekeyfile`).
8. **Ofereça ferramentas**: inspeção de identidade (`corflags`, `sn`-like), verificação de assinatura, modo de teste/fake.
9. **Para compatibilidade**: se você já publicou assemblies com uma identidade, trocar é *breaking change* — planeje a identidade desde o início.

## Referências

- `docs/project/strong-name-signing.md` — motivação, FAQ, problemas.
- `docs/project/public-signing.md` — public signing, `CorFlags`, MSBuild, FakeSign.
- `src/coreclr/inc/corhdr.h` — `COMIMAGE_FLAGS_STRONGNAMESIGNED`, `StrongNameSignature`, `CorAssemblyFlags` (`afPublicKey`, `afRetargetable`, `afPA_*`).
- `src/coreclr/inc/strongnameinternal.h`, `src/coreclr/md/runtime/strongnameinternal.cpp` — `PublicKeyBlob`, `StrongNameToken`, `StrongNameTokenFromPublicKey`, `g_coreLibPublicKey`.
- `src/coreclr/inc/pedecoder.h`/`pedecoder.inl` — `HasStrongNameSignature`, `IsStrongNameSigned`, `CheckStrongNameSignature`.
- `src/coreclr/binder/assemblyname.cpp`, `src/coreclr/binder/utils.cpp` — identidade e comparação no binder.
- `src/coreclr/vm/baseassemblyspec.cpp` — reconhecimento de satélites de CoreLib por chave.
- `src/coreclr/tools/aot/crossgen2/Program.cs`, `Crossgen2RootCommand.cs` — `--compositekeyfile`, `IsValidPublicKey`.
- ECMA-335 Partição II (seção de assemblies e identidade; tabelas `Assembly` e `AssemblyRef`; região StrongNameSignature no formato PE/CLI).

## Apêndice A — Passo a passo de identidade

1. O autor gera um par de chaves RSA. A **chave pública** vai para a tabela `Assembly` (coluna `PublicKey`, com `afPublicKey` na flag da referência quando for chave completa).
2. O compilador escreve a versão (4 inteiros), cultura e nome.
3. Ao assinar, o ferramental calcula o hash do assembly (excluindo a região `StrongNameSignature`), assina com a chave privada e grava o resultado na região apontada por `IMAGE_COR20_HEADER.StrongNameSignature`; liga `COMIMAGE_FLAGS_STRONGNAMESIGNED` (0x8).
4. Quem referencia esse assembly grava no `AssemblyRef`: nome, versão, cultura e **token** (8 bytes derivados da chave pública via SHA-1, `StrongNameTokenFromPublicKey`).
5. Na carga, o binder (`AssemblyName::Equals`) compara nome simples (sem diferenciar maiúsculas), cultura, token e, conforme flags de inclusão, versão/arquitetura/retargetable.

Etapa 3 descreve o modelo clássico; no .NET moderno, ver a nota sobre `CheckStrongNameSignature` (só valida o diretório). Em **public signing**, a etapa 3 é substituída por apenas ligar o bit.

## Apêndice B — Exemplo de configuração MSBuild (conforme `public-signing.md`)

```xml
<PropertyGroup>
  <AssemblyOriginatorKeyFile>MyKey.snk</AssemblyOriginatorKeyFile> <!-- contém a chave pública (public signing) -->
  <PublicSign>True</PublicSign>
  <DelaySign>False</DelaySign> <!-- precisa ser False quando PublicSign é True -->
</PropertyGroup>
```

Observação: o doc diz que, com `/publicsign`, os compiladores C#/VB (a partir do VS 2015 Update 2) gravam a chave pública e ligam o bit sem assinar. A propriedade `AssemblyOriginatorKeyFile` é a convenção do SDK .NET para apontar o arquivo de chave e não está citada no texto do repositório; confirme no SDK em uso.

## Apêndice C — Quando ainda importa strong name em 2025+ (segundo os docs do repo)

- Para consumidores no **.NET Framework** que exigem assemblies assinados ou precisam instalar no GAC.
- Para **bibliotecas públicas** que não querem excluir consumidores strong-named (a recomendação do doc).
- Para **compatibilidade de identidade de tipo** com assemblies já publicados (remover é breaking change).
- Para **servicing app-local** no .NET Framework.

Não importa para: segurança/sandboxing (o próprio doc diz que sandboxing por AppDomain "is no longer guaranteed and is not recommended"), nem para versionar em .NET Core (política de binding relaxada).

## Apêndice D — Esboço para uma linguagem nova (modelo mínimo recomendado)

```
Identidade do módulo  = { nome, versão, cultura?, token(chave pública)? }
Token                 = últimos N bytes de H(chavePública)         // N=8 no .NET com SHA-1
Referência            = { nome, versão mínima, token? }
Binding               = nome + token iguais; versão_carregada >= versão_pedida
Assinatura            = opcional; checada por ferramenta de distribuição, não no carregamento
Public/delay signing  = suportado: bit "assinado" ligado sem assinatura real (builds de terceiros)
```

Pontos para decidir:

- **Hash do token**: o .NET usa SHA-1 por herança; hoje prefira um hash moderno, mantendo token curto apenas como *identificador*.
- **Revogação e rotação de chaves**: strong names não têm; planeje identidade com mecanismo de migração (tabela de redirecionamento).
- **Onde validar**: ferramentas (instalador/registry de pacotes) em vez de no loader.
- **Relação com assinatura de pacote/arquivo**: mantenha-as separadas.

## Apêndice E — Estruturas e constantes de referência

| Item | Valor / forma | Arquivo |
|---|---|---|
| `COMIMAGE_FLAGS_STRONGNAMESIGNED` | `0x00000008` | `src/coreclr/inc/corhdr.h` |
| `IMAGE_COR20_HEADER.StrongNameSignature` | `IMAGE_DATA_DIRECTORY {VirtualAddress, Size}` | `corhdr.h` |
| `afPublicKey` | `0x0001` | `corhdr.h` (`CorAssemblyFlags`) |
| `afRetargetable` | `0x0100` | `corhdr.h` |
| `StrongNameToken::SIZEOF_TOKEN` | 8 bytes | `strongnameinternal.h` |
| `PublicKeyBlob` | `SigAlgID`, `HashAlgID`, `cbPublicKey`, `PublicKey[]` | `strongnameinternal.h` |
| Cálculo do token | SHA-1 da chave; últimos 8 bytes, ordem invertida | `md/runtime/strongnameinternal.cpp` |
| `mdtAssembly` / `mdtAssemblyRef` | `0x20000000` / `0x23000000` | `corhdr.h` |

## Apêndice F — Perguntas frequentes

- **O runtime moderno verifica a assinatura?** No código inspecionado, `PEDecoder::CheckStrongNameSignature` só valida a região (diretório) e não a criptografia; a doc de public signing reforça que o carregamento funciona "in any context that doesn't require validating the strong-name signature".
- **Por que manter strong name então?** Compatibilidade de identidade e servicing (`strong-name-signing.md`).
- **Preciso de strong name na minha aplicação?** O doc diz que, em geral, não; é recomendado para bibliotecas públicas.
- **O que acontece se eu não tenho a chave privada de um projeto OSS?** Public signing permite builds com a mesma identidade (token) sem a assinatura real.
