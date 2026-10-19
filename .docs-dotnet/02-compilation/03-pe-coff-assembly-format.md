# Formato PE/COFF de um assembly gerenciado (cabeçalho CLI)

## Mecanismo no .NET

Um assembly .NET (`.dll`/`.exe`) é um arquivo **PE/COFF** válido (o mesmo formato dos executáveis nativos do Windows), com uma extensão: uma das 16 *data directories* do cabeçalho opcional aponta para o **cabeçalho CLI** (`IMAGE_COR20_HEADER`). O comentário `#ManagedHeader` em `src/coreclr/inc/corhdr.h` resume: *"Managed code has defined one of these entries (the 14th see IMAGE_DIRECTORY_ENTRY_COMHEADER) and the RVA points [to] the IMAGE_COR20_HEADER."* A especificação é ECMA-335 Partição II §25 (formato de arquivo PE); o addendum do repo é `docs/design/specs/PE-COFF.md`.

### Camadas

```
+--------------------------------------+
| DOS header / PE signature            |  (stub "This program cannot be run in DOS mode")
| COFF file header                     |
| Optional header (PE32 ou PE32+)      |  -> DataDirectory[14] = COM Descriptor (CLI header)
| Section table (.text, .rsrc, .reloc) |
+--------------------------------------+
| .text:                               |
|   IAT / import stub (legado)         |
|   IMAGE_COR20_HEADER  (72 bytes)     |
|   IL dos métodos (cada um com header)|
|   Metadata root (streams)            |
|   Recursos gerenciados               |
|   Strong name signature              |
|   VTableFixups / EAT jumps (opcional)|
|   ManagedNativeHeader (R2R) (opcional)|
+--------------------------------------+
```

### `IMAGE_COR20_HEADER`

Definição real em `src/coreclr/inc/corhdr.h` (linhas ~209–247):

```c
typedef struct IMAGE_COR20_HEADER {
    uint32_t cb;                                  // tamanho do header
    uint16_t MajorRuntimeVersion, MinorRuntimeVersion;
    IMAGE_DATA_DIRECTORY MetaData;                // RVA+tamanho da metadata (arquivo 02)
    uint32_t Flags;                               // COMIMAGE_FLAGS_*
    union { uint32_t EntryPointToken;             // token de MethodDef (ponto de entrada gerenciado)
            uint32_t EntryPointRVA; };            // RVA nativo se COMIMAGE_FLAGS_NATIVE_ENTRYPOINT
    IMAGE_DATA_DIRECTORY Resources;               // blob de recursos gerenciados
    IMAGE_DATA_DIRECTORY StrongNameSignature;     // assinatura strong name (arquivo 11)
    IMAGE_DATA_DIRECTORY CodeManagerTable;        // obsoleto, não usado
    IMAGE_DATA_DIRECTORY VTableFixups;            // gerenciado exposto como entry points nativos
    IMAGE_DATA_DIRECTORY ExportAddressTableJumps;
    IMAGE_DATA_DIRECTORY ManagedNativeHeader;     // null em IL puro; READYTORUN_HEADER em R2R
} IMAGE_COR20_HEADER;
```

Flags (`COMIMAGE_FLAGS_*`, `corhdr.h` ~linha 129):

| Flag | Valor | Significado |
|---|---|---|
| `ILONLY` | `0x00000001` | Apenas IL (sem código nativo embutido) |
| `32BITREQUIRED` | `0x00000002` | Originalmente "x86 apenas" |
| `IL_LIBRARY` | `0x00000004` | Imagem de biblioteca IL — **usada pelas imagens R2R** (`readytorun-format.md`) |
| `STRONGNAMESIGNED` | `0x00000008` | Possui assinatura de strong name |
| `NATIVE_ENTRYPOINT` | `0x00000010` | `EntryPointRVA` é nativo |
| `TRACKDEBUGDATA` | `0x00010000` | — |
| `32BITPREFERRED` | `0x00020000` | Plataforma neutra, mas prefere 32 bits |

`corhdr.h` explica (linhas ~82–120) que `32BITREQUIRED` e `32BITPREFERRED` formam um par com macros (`COR_IS_32BIT_REQUIRED`, `COR_SET_32BIT_REQUIRED`...) e **não devem ser manipulados diretamente** por compatibilidade com runtimes pré-4.5.

O ponto de entrada de um `.exe` é um token `MethodDef` (`0x06xxxxxx`); se `NATIVE_ENTRYPOINT` está setada, é um RVA nativo (obsoleto para DLLs: "use modules constructors instead" — ver arquivo 10).

### Corpo de método (IL method header)

Cada `MethodDef.RVA` aponta para um `IMAGE_COR_ILMETHOD`: um cabeçalho *tiny* (1 byte, tamanho de código < 64, sem locais nem EH, pilha máx. 8) ou *fat* (12 bytes: flags, `MaxStack`, `CodeSize`, `LocalVarSigTok` — token de `StandAloneSig`), seguido do IL e das seções de dados com cláusulas de exceção. (ECMA-335 II.25.4.)

### Como o runtime reconhece e valida

`src/coreclr/inc/pedecoder.h` expõe a classe `PEDecoder`, com métodos reais:

- `HasCorHeader()`, `GetMetadata(...)`, `IsILOnly()`, `HasManagedEntryPoint()`, `HasNativeEntryPoint()`, `IsPlatformNeutral()`, `HasReadyToRunHeader()`, `HasStrongNameSignature()`, `IsStrongNameSigned()`, `CheckStrongNameSignature()`.

`HasReadyToRunHeader()` verifica `ManagedNativeHeader` + a assinatura `0x00525452` ("RTR") — ver `readytorun-format.md`, "READYTORUN_HEADER::Signature".

### Addendum do repo: determinismo e Debug Directory

`docs/design/specs/PE-COFF.md` complementa a spec do PE:

- **PE determinístico**: o `TimeDateStamp` do COFF deixa de ser tempo e passa a ser derivado de hash do conteúdo; "The content of the file can be reproduced exactly (bit for bit) given the same inputs".
- **Entradas do Debug Directory** usadas por .NET:
  - Tipo 2 *CodeView* (`RSDS`, GUID + Age + caminho do `.pdb`); com `Minor=0x504d` indica Portable PDB e o *PDB ID* = GUID(16B) + TimeDateStamp(4B).
  - Tipo 16 *Deterministic* (sem dados).
  - Tipo 17 *Embedded Portable PDB* (assinatura `MPDB`, tamanho descomprimido, imagem comprimida em Deflate).
  - Tipo 19 *PDB Checksum* (SHA256/384/512 do conteúdo do PDB).
  - Tipo 21 *R2R PerfMap* (assinatura `R2RM`, 16B de signature, versão, caminho do `.r2rmap`).

### Outros contêineres

- **R2R composto** usa PE como "envelope nativo" em todas as plataformas até .NET 10; `docs/design/coreclr/botr/readytorun-platform-native-envelope.md` descreve o plano para Mach-O (`--obj-format macho`) no .NET 11 (somente composite). O header R2R é encontrado pelo símbolo exportado `RTR_HEADER`.
- **Webcil** (`src/coreclr/inc/webcildecoder.h`, `docs/design/mono/webcil.md`): contêiner alternativo para assemblies ECMA-335 usado no browser/WebAssembly, que "replaces PE headers with a minimal Webcil header and section table", embrulhado em um módulo `.wasm`. Motivação documentada: firewalls/antivírus que bloqueiam `.dll` com conteúdo PE.
- **Imagens de Native AOT** não são assemblies: são executáveis nativos da plataforma (COFF/ELF/Mach-O) gerados por link (`docs/workflow/building/coreclr/nativeaot.md`).

## Por quê

- **Reuso do ecossistema de SO**: usar PE significou que o loader do Windows, depuradores, ferramentas de assinatura (Authenticode) e `dumpbin` funcionam sem mudança. Um arquivo gerenciado ainda tem entrada PE (em versões antigas, um stub de import para `mscoree.dll`).
- **Separação limpa**: PE = contêiner (seções, RVAs, relocs); CLI header = índice para o conteúdo gerenciado; metadata = descrição. Cada camada evolui separadamente.
- **Mapeamento em memória (mmap)**: tudo é endereçado por RVA, então o runtime mapeia o arquivo e usa ponteiros relativos sem desserializar.
- **Extensão por ponteiro**: `ManagedNativeHeader` permite anexar estruturas extras (NGEN, depois R2R) sem alterar o formato base — a razão pela qual R2R consegue ser "um assembly com código nativo anexado" (`readytorun-overview.md`: "A CLI file has a well-defined header structure, and that header already has a field that can point to 'additional information'").

## Trade-offs

| Escolha | Vantagem | Custo |
|---|---|---|
| Envelope PE em todas as plataformas | Um único formato para todos os SOs | Em Linux/macOS o loader nativo não é usado; o runtime faz mapeamento e relocation próprios (`readytorun-platform-native-envelope.md`: "Non-Windows platforms therefore load a PE file with the .NET loader performing the required fixups") |
| RVAs em vez de offsets de arquivo | Alinhamento de seção independente do arquivo | Conversão RVA↔offset em toda ferramenta |
| Header CLI fixo de 72 bytes | Simples de ler | Evoluir exige campos novos em estruturas apontadas (R2R header) e não no próprio header |
| IL e metadata no mesmo arquivo | Autossuficiente | Arquivos de IL carregam metadata mesmo quando só o código nativo é usado |
| Bits de "arquitetura" (`32BITREQUIRED`/`PREFERRED`) | Controlam bitness | Semântica confusa legada; macros obrigatórias para manipular o par |
| Conteúdo identificado por PDB GUID+Age/ID | Casamento de símbolos | Depende de caminhos/servidores de símbolos |

## Como aplicar na sua linguagem

1. **Escolha um contêiner**: reutilizar PE/ELF/Mach-O dá ferramentas "de graça"; um formato próprio (como Webcil) simplifica o runtime mas exige ferramentas novas. Se o alvo é WebAssembly, envolva o payload num módulo `.wasm` como o Webcil.
2. **Tenha um cabeçalho de "managed" pequeno**, com:
   - versão do formato (major/minor),
   - ponteiro para metadata,
   - flags de imagem (IL puro, assinado, biblioteca, arquitetura),
   - token/RVA do ponto de entrada,
   - ponteiros para recursos e assinatura,
   - **um ponteiro de extensão** (equivalente a `ManagedNativeHeader`) para anexar código pré-compilado ou outras tabelas sem mudar a versão do formato base.
3. **Endereçamento relativo (RVA ou offset em seção)** para permitir mmap e relocation lazy.
4. **Corpos de método com cabeçalho compacto** (tiny/fat): 1 byte para métodos triviais economiza muito em bibliotecas com muitos getters.
5. **Suporte a build determinístico desde o início**: carimbo de tempo derivado de hash; entrada explícita "Deterministic". Facilita cache de build, reprodutibilidade e assinatura.
6. **Casar binário ↔ símbolos por ID** (GUID + checksum do PDB) gravado no binário, não por caminho.
7. **Versione estruturas apontadas com maior/menor**: regra do R2R — "The runtime should be able to successfully execute native code from images of version 2.9 [if it supports 2.3]... should refuse ... 3.0" (`readytorun-format.md`).
8. **Defina uma validação barata** (`HasCorHeader`, tamanho, assinatura mágica) antes de qualquer parsing mais profundo — reduz superfície de ataque.

## Referências

- `src/coreclr/inc/corhdr.h` — `IMAGE_COR20_HEADER`, `COMIMAGE_FLAGS_*`, comentário `#ManagedHeader`, `COR_VTABLE_*`.
- `src/coreclr/inc/pedecoder.h` / `pedecoder.inl` — `PEDecoder`.
- `src/coreclr/inc/webcildecoder.h`, `docs/design/mono/webcil.md` — Webcil.
- `docs/design/specs/PE-COFF.md` — determinismo e Debug Directory (tipos 2, 16, 17, 19, 21).
- `docs/design/coreclr/botr/readytorun-format.md` — "PE Headers and CLI Headers", `READYTORUN_HEADER`.
- `docs/design/coreclr/botr/readytorun-platform-native-envelope.md` — plano Mach-O.
- ECMA-335 Partição II §25 (PE file format, CLI header, method header), Partição II §24.

## Apêndice A — Ordem de validação sugerida por `PEDecoder`

Com base nos métodos de `src/coreclr/inc/pedecoder.h`, uma sequência típica de verificação ao abrir uma imagem é:

1. `CheckNTHeaders()` / cabeçalho PE básico (assinatura, tamanhos de seção);
2. `HasCorHeader()` — existe a *data directory* nº 14 e o `IMAGE_COR20_HEADER` cabe na imagem?
3. `GetMetadata()` — diretório de metadata dentro dos limites;
4. `IsILOnly()` / `IsPlatformNeutral()` / `HasNativeEntryPoint()` — decide como carregar (IL puro × com código nativo);
5. `HasReadyToRunHeader()` — se `ManagedNativeHeader` está presente e a assinatura `RTR` confere, o runtime consulta a versão (aceita minor maior, recusa major maior) e as flags (`READYTORUN_FLAG_*`);
6. `HasStrongNameSignature()` — apenas verifica a presença/validade do diretório (a assinatura em si não é validada em carga no .NET moderno — arquivo 11).

Nota: os contratos `CONTRACT_CHECK` mostram que `CheckStrongNameSignature()` tem como pré-condições `CheckNTHeaders()`, `HasCorHeader()` e `HasStrongNameSignature()`.

## Apêndice B — Mapa de ponteiros do header CLI

```
IMAGE_COR20_HEADER
 ├─ MetaData ───────────────► metadata root ──► streams (#~, #Strings, #US, #GUID, #Blob)
 │                                              └─ MethodDef.RVA ──► IMAGE_COR_ILMETHOD (tiny/fat) + IL + EH
 ├─ Resources ──────────────► blob de recursos gerenciados (ManifestResource.Offset indexa aqui)
 ├─ StrongNameSignature ────► região reservada (arquivo 11)
 ├─ VTableFixups ───────────► tabela de slots (gerenciado exportado como nativo, `COR_VTABLE_*`)
 ├─ ExportAddressTableJumps ► thunks de salto (`IMAGE_COR_EATJ_THUNK_SIZE = 32`)
 └─ ManagedNativeHeader ────► (R2R) READYTORUN_HEADER ──► seções (arquivo 05)
```

`COR_VTABLE_32BIT=0x01`, `COR_VTABLE_64BIT=0x02`, `COR_VTABLE_FROM_UNMANAGED=0x04`, `COR_VTABLE_CALL_MOST_DERIVED=0x10` são flags dos *V-table fixups* (corhdr.h): servem a assemblies "mixed-mode" que exportam métodos gerenciados para chamadores nativos (C++/CLI), ver `docs/design/coreclr/botr/mixed-mode.md`.

## Apêndice C — Decisões de formato a copiar ou evitar

| Decisão do .NET | Copiar? | Motivo |
|---|---|---|
| Endereçamento por RVA com `IMAGE_DATA_DIRECTORY {VirtualAddress, Size}` | Sim | Pares simples e uniformes; permite seções reordenáveis |
| Campo `cb` (tamanho do header) como primeiro campo | Sim | Permite crescer o header em versões futuras |
| Campos de versão do header (`MajorRuntimeVersion`/`MinorRuntimeVersion`) | Sim, mas use-os de verdade | O CLR raramente os usa; a evolução real foi feita por flags e por headers apontados (R2R) |
| Campo `CodeManagerTable` (obsoleto, "Deprecated, not used") | Não | Exemplo de campo morto herdado; reserve em vez de nomear |
| Dois campos de 32 bits (`32BITREQUIRED`/`32BITPREFERRED`) com semântica combinada | Não | Corhdr.h precisa de macros para ocultar a combinação |
| União `EntryPointToken`/`EntryPointRVA` controlada por flag | Com cuidado | Útil, mas confunde; prefira dois campos |
