# PDB / informação de debug e sequence points

## Mecanismo no .NET

Informação de debug de código gerenciado tem **duas camadas** distintas:

1. **Debug info "fonte ↔ IL"**, produzida pelo compilador de linguagem (Roslyn) e guardada em um arquivo de símbolos (PDB) ou embutida no assembly: documentos (arquivos-fonte), *sequence points* (IL offset ↔ linha/coluna), escopos de variáveis locais, constantes, imports, informações de state machines (async/iterators), etc.
2. **Debug info "IL ↔ nativo"**, produzida em tempo de compilação nativa (JIT, crossgen2, ILC): mapeia offsets IL para offsets de código nativo e descreve onde cada variável vive (registrador/pilha) em cada intervalo.

O depurador combina as duas para mostrar "esta linha do código-fonte ↔ este endereço de máquina".

### Camada 1: Portable PDB

Especificação: `docs/design/specs/PortablePdb-Metadata.md` ("Portable PDB v1.0"). É um **arquivo de metadata ECMA-335** (mesmo formato de tabelas do arquivo 02) com tabelas adicionais, multiplataforma e compacto, ao contrário do PDB do Windows (formato MSF proprietário).

**Streams**: `#Pdb` (ID de 20 bytes: GUID+stamp, token do entry point, tamanhos de tabelas do módulo associado) e `#~` com as tabelas de debug (as tabelas do assembly ficam no assembly; o PDB só usa tokens delas).

**Tabelas** (IDs confirmados em `src/coreclr/inc/metamodelpub.h` e na spec):

| ID | Tabela | Conteúdo |
|---|---|---|
| 0x30 | Document | nome (blob de partes de caminho), hash do arquivo (algoritmo GUID + bytes), linguagem (GUID) |
| 0x31 | MethodDebugInformation | `Document` + `SequencePoints` (blob). Uma linha por `MethodDef`, indexável pelo RID do método |
| 0x32 | LocalScope | escopos léxicos: `Method`, `ImportScope`, `VariableList`, `ConstantList`, `StartOffset`, `Length` |
| 0x33 | LocalVariable | atributos, índice, nome |
| 0x34 | LocalConstant | nome + assinatura (`LocalConstantSig`) |
| 0x35 | ImportScope | `using`s e aliases (`Imports` blob) |
| 0x36 | StateMachineMethod | `MoveNextMethod` ↔ `KickoffMethod` (async/iterator) |
| 0x37 | CustomDebugInformation | `Parent` (HasCustomDebugInformation, 5 bits de tag), `Kind` (GUID), `Value` (blob). Extensível por ferramentas |

Registros custom definidos pela spec (cada um identificado por GUID): hoisted local scopes de state machines, dynamic locals (C#), default namespace (VB), **EnC local slot map**, **EnC lambda and closure map**, **Embedded Source**, **Source Link**, compilation metadata references, compilation options, type definition document.

### Sequence points (formato)

Um *sequence point* é a quíntupla `(IL offset, start line, start col, end line, end col)` + documento. Regras da spec:

- **Hidden sequence point**: `StartLine = EndLine = 0xFEEFEE`, colunas 0. Marca trechos do IL sem correspondência em código-fonte (o depurador não para ali).
- IL offset no intervalo `[0, 0x20000000)` e **estritamente crescente**.
- `StartLine`, `EndLine` em `[0, 0x20000000)` e ≠ `0xFEEFEE` (para não-hidden); colunas em `[0, 0x10000)`; `EndLine ≥ StartLine`; se mesma linha, `EndColumn > StartColumn`.

**Blob** (`MethodDebugInformation.SequencePoints`):

```
Blob ::= header SequencePointRecord (SequencePointRecord | document-record)*
header: LocalSignature (RID de StandAloneSig, comprimido)
        [InitialDocument]   (se o método abrange vários documentos)
```

Cada registro usa **delta** com inteiros comprimidos (arquivo 04): `δILOffset` (diferença do anterior, não-zero), `ΔLines`, `ΔColumns` (sem sinal se 0 linhas; com sinal caso contrário), `δStartLine`/`δStartColumn` (com sinal, relativos ao último ponto *não-hidden*). Pontos hidden têm `ΔLine = ΔColumn = 0`. Um `document-record` (`δILOffset = 0`) troca de documento. O campo `LocalSignature` é redundante com o IL mas evita paginar o corpo do método.

### Como o PDB é ligado ao binário

`docs/design/specs/PE-COFF.md` (arquivo 03): entrada **CodeView** (tipo 2, `RSDS`, GUID+Age+caminho) — para Portable PDB, `Minor=0x504d`, `Age=1` e o ID = GUID(16B)+`TimeDateStamp`(4B); entrada **Embedded Portable PDB** (tipo 17, `MPDB`, Deflate) quando o PDB vai dentro do assembly; **PDB Checksum** (tipo 19, SHA256/384/512 do PDB com o ID zerado); **Deterministic** (tipo 16). O PE determinístico deriva GUID/timestamp de hash do conteúdo do PDB.

### Camada 2: mapas IL ↔ nativo (`cordebuginfo.h`)

`src/coreclr/inc/cordebuginfo.h` define `ICorDebugInfo`, preenchida pelo JIT e consumida pelo debugger (`src/coreclr/debug/ee`):

- **`OffsetMapping { nativeOffset, ilOffset, source }`**: `ilOffset` pode ser um valor especial: `NO_MAPPING = -1`, `PROLOG = -2`, `EPILOG = -3`.
- **`BoundaryTypes`** pedidos pelo debugger ao JIT: `STACK_EMPTY_BOUNDARIES (1)` (pontos onde a pilha de avaliação está vazia), `NOP_BOUNDARIES (2)`, `CALL_SITE_BOUNDARIES (4)`; `DEFAULT_BOUNDARIES` é a união. Isto se relaciona ao IL: **só se pode parar/editar com segurança onde a pilha de avaliação está vazia**.
- **`SourceTypes`** (bit flags): `SEQUENCE_POINT (1)`, `STACK_EMPTY (2)`, `CALL_SITE (4)`, `NATIVE_END_OFFSET_UNKNOWN (8, epílogo)`, `CALL_INSTRUCTION (0x10)`, `ASYNC (0x20)`. O comentário explica: um sequence point também pode ser stack-empty e call site; o debugger testa `source & SEQUENCE_POINT`.
- **`NativeVarInfo { startOffset, endOffset, callReturnValueILOffset, varNumber, loc }`** com `VarLoc.vlType`: `VLT_REG`, `VLT_REG_BYREF`, `VLT_REG_FP`, `VLT_STK`, `VLT_STK_BYREF`, `VLT_REG_REG`, `VLT_REG_STK`, `VLT_STK_REG`, `VLT_STK2`, `VLT_FPSTK`, `VLT_FIXED_VA`. Isto permite que uma variável mude de lugar ao longo do método (código otimizado).
- **`InlineTreeNode`** (árvore de inlining) e informações de ASYNC.
- Há `[cDAC]` marcando valores espelhados no código gerenciado do "contract-based data access".

Para a combinação: o debugger usa o PDB para saber "linha X ↔ IL offset Y" (via sequence points) e o mapa do JIT para "IL offset Y ↔ nativo Z".

### Debug info em R2R e Native AOT

- **R2R**: seção `DebugInfo` (105) com a "mesma codificação do NGEN" ("TODO: Document ... should not be required when debuggers are able to handle debug info stored separately"). Crossgen2 gera PDB/`.r2rmap` (`--pdb`, `--perfmap`, `ILCompiler.Diagnostics` com `PdbWriter.cs`, `PerfMapWriter.cs`, `ISymNGenWriter.cs`); o formato do perfmap está em `r2r-perfmap-format.md` (linhas `RVA(4B) tamanho(2B) nome`, com cabeçalho de assinatura de 16 bytes/versão/OS/arquitetura/ABI). `--strip-debug-info` remove. O crossgen2 lê o `DebuggableAttribute` do assembly de entrada para decidir se otimiza ou gera código depurável quando nenhum nível foi pedido na linha de comando (`AssemblyExtensions.HasOptimizationsDisabled`, usado em `Program.cs`).
- **Native AOT**: o ILC emite debug info **nativa**: CodeView no Windows, DWARF no Linux/macOS (`nativeaot.md`). O doc enfatiza: "can be debugged with native debuggers and have full-fidelity access to locals, and stepping information". `IlcDumpGeneratedIL` mapeia debug info para o `.il` dos corpos sintetizados, permitindo *stepping* neles. O `StartupCodeMainMethod` marca `MarkDebuggerStepThroughPoint()` no Windows.

### `DebuggableAttribute`

`System.Diagnostics.DebuggableAttribute` (`src/libraries/System.Private.CoreLib/src/System/Diagnostics/DebuggableAttribute.cs`) com `DebuggingModes.Default (0x1)` e `DisableOptimizations`: o JIT/crossgen2 decidem otimização a partir dele. Em `corhdr.h`: `afEnableJITcompileTracking = 0x8000` e `afDisableJITcompileOptimizer = 0x4000` ("From DebuggableAttribute") dentro de `CorAssemblyFlags`.

### Edit and Continue

As tabelas `ENCLog`/`ENCMap` (arquivo 02) e os registros custom de EnC no Portable PDB (local slot map, lambda/closure map) permitem recompilar partes de um método em execução. `NOP_BOUNDARIES` e `STACK_EMPTY` evitam colocar breakpoints de EnC onde a pilha não está vazia.

## Por quê

- **Separar símbolos do binário**: binários menores; símbolos distribuídos por servidor de símbolos (Source Link, símbolos embutidos).
- **Formato aberto e portátil**: Portable PDB reutiliza o formato de metadata (leitor existente), é pequeno (deltas + inteiros comprimidos) e funciona em Linux/macOS.
- **Estabilidade por sequence points**: o depurador só precisa de poucos pontos estáveis por método; hidden points suprimem código gerado (ex.: state machines).
- **Dois níveis**: a linguagem não precisa saber de registradores; o backend não precisa saber de linhas de fonte.
- **Reprodutibilidade**: ID/checksum deterministas.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| PDB separado | Binário enxuto | Pode faltar na hora do crash; precisa de casamento por ID + servidor de símbolos |
| PDB embutido (`MPDB`) | Sempre disponível | Binário maior (Deflate mitiga) |
| Delta + comprimido | Compacto | Decoder necessário; leitura sequencial |
| Sequence points só onde pilha vazia | Simples/seguro | Granularidade menor; expressões multiline podem ter paradas pouco intuitivas |
| Código otimizado | Rápido | Variáveis podem "sumir" (`NativeVarInfo` por intervalos) e linhas se reordenam |
| R2R com debug info NGEN-like | Compat com debugger | Infla a imagem; R2R não é código "debugável" |
| AOT com debug nativo | Ferramentas nativas | Expressões avaliadas/EnC gerenciados não disponíveis como no JIT |

## Como aplicar na sua linguagem

1. **Decida o modelo de debug desde o início**: onde o "mapa de fonte" mora (arquivo separado, embutido) e como se casa com o binário (GUID + checksum, não caminho).
2. **Use sequence points em IL offsets** (não em endereços nativos): você obtém portabilidade entre JIT/AOT/interpretador; o backend traduz para nativo.
3. **Tenha um conceito de "hidden"** para código sintetizado (lambdas, state machines, desugaring) — evita que o depurador pare em código que o usuário não escreveu.
4. **Codifique com deltas e inteiros variáveis**; mantenha documentos numa tabela e referencie por RID.
5. **Escopos de variáveis e constantes** como tabelas (LocalScope/LocalVariable/LocalConstant) com intervalos IL; permita ao backend mover a variável (VarLoc por intervalo).
6. **Extensibilidade**: uma tabela genérica `(parent, kind GUID, blob)` como `CustomDebugInformation` evita mudar o formato quando a linguagem ganha novos recursos (state machines, closures, source link, código fonte embutido).
7. **Peça ao backend limites de parada** (`STACK_EMPTY`, `CALL_SITE`, `NOP`) se sua IR é de pilha: determinam onde breakpoints são seguros.
8. **Para AOT**, emita debug info nativo padrão (DWARF/CodeView) e forneça um modo para depurar o IL sintetizado.
9. **Fonte embutida e Source Link** facilitam depuração remota; reserve registros para eles.
10. **Separe o formato de perfmap/símbolos de profiling** do de debug: R2R usa `.r2rmap` simples para profilers.

## Referências

- `docs/design/specs/PortablePdb-Metadata.md` — tabelas 0x30–0x37, blob de sequence points, registros custom.
- `docs/design/specs/PE-COFF.md` — entradas CodeView, Embedded PDB, PDB Checksum, Deterministic, R2R PerfMap.
- `src/coreclr/inc/cordebuginfo.h` — `OffsetMapping`, `BoundaryTypes`, `SourceTypes`, `NativeVarInfo`, `VarLoc`.
- `src/coreclr/inc/metamodelpub.h`, `src/coreclr/md/inc/portablepdbmdds.h`, `pdbheap.h` — suporte a tabelas de PDB no `md`.
- `src/coreclr/debug/ee/` — consumo no debugger (`debugger.cpp`, `controller.cpp`, `functioninfo.cpp`).
- `src/coreclr/tools/aot/ILCompiler.Diagnostics/` — `PdbWriter.cs`, `PerfMapWriter.cs`, `ISymNGenWriter.cs`.
- `docs/design/coreclr/botr/r2r-perfmap-format.md`, `readytorun-format.md` (seção `DebugInfo`).
- `docs/design/specs/Ecma-335-Augments.md` — "Debug Interchange Format".
- ECMA-335 Partição III (`nop`) e o anexo "Debug Interchange Format" (referido em `Ecma-335-Augments.md`).

## Apêndice A — Exemplo de decodificação de sequence points

Suponha um método com três pontos (todos num único documento, `Document` preenchido em `MethodDebugInformation`):

| # | IL offset | Linha/Col início | Linha/Col fim | Hidden? |
|---|---|---|---|---|
| 1 | 0 | 10:5 | 10:6 | não |
| 2 | 1 | 11:9 | 11:25 | não |
| 3 | 7 | `0xFEEFEE`:0 | `0xFEEFEE`:0 | sim |

Codificação conforme a spec (todos inteiros comprimidos, arquivo 04):

```
header:        LocalSignature (RID do StandAloneSig)
registro 1:    δILOffset=0           ΔLines=0   ΔColumns=1 (sem sinal, não zero)  StartLine=10 (sem sinal)  StartColumn=5 (sem sinal)
registro 2:    δILOffset=1           ΔLines=0   ΔColumns=16                      δStartLine=+1 (com sinal)  δStartColumn=+4 (com sinal)
registro 3:    δILOffset=6 (7-1)     ΔLine=0    ΔColumn=0                         (hidden: nenhum campo de linha/coluna)
```

Observações extraídas da spec: o primeiro registro não-hidden carrega valores absolutos de linha/coluna (sem sinal); os seguintes usam deltas com sinal em relação ao último **não-hidden**; `δILOffset` é sempre não zero exceto no `document-record`.

## Apêndice B — Relação entre os três arquivos de uma build com símbolos

```
MyApp.dll                      (assembly: IL + metadata; Debug Directory: CodeView + Checksum [+ Embedded PDB])
MyApp.pdb                      (Portable PDB: Document, MethodDebugInformation, LocalScope, ...)
MyApp.ni.r2rmap (opcional)     (R2R PerfMap, ligado por Debug Directory tipo 21)
```

Casamento: GUID(16B) + TimeDateStamp(4B) da entrada CodeView = PDB ID no stream `#Pdb`; o checksum (SHA256/384/512) calculado com o PDB ID zerado permite validar integridade.

## Apêndice C — O que um depurador precisa de uma nova linguagem

1. **Mapa fonte↔IL** (sequence points, com hidden).
2. **Escopos e variáveis** (nomes, slots, intervalos IL; constantes).
3. **Identificação do tipo do estado de uma máquina de estados** se a linguagem tem async/iteradores (`StateMachineMethod`, hoisted locals).
4. **Mapa IL↔nativo** do backend (offsets com `PROLOG`/`EPILOG`/`NO_MAPPING`), mais `VarLoc` por intervalo.
5. **Metadados de linguagem** extensíveis (GUID + blob) para expressões, closures, `dynamic`.
6. **Fontes**: caminho + hash do arquivo (para validar), fonte embutida e URL de Source Link.
7. **Casamento seguro binário↔símbolos** (ID + checksum).
