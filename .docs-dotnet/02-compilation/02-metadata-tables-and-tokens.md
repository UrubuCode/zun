# Tabelas de metadata e tokens (TypeDef / MethodDef / MemberRef / TypeRef)

## Mecanismo no .NET

Todo assembly gerenciado carrega **metadata**: um "banco de dados" relacional embutido que descreve tipos, métodos, campos, parâmetros, atributos, referências a outros assemblies etc. O IL (arquivo 01) nunca menciona nomes ou offsets: ele referencia entidades por **tokens** de 32 bits que indexam essas tabelas. A especificação é ECMA-335 Partição II (§22 tabelas, §24 formato físico). O repositório documenta correções em `docs/design/specs/Ecma-335-Augments.md`.

### Token = (tabela, linha)

`src/coreclr/inc/corhdr.h` define o enum `CorTokenType` com o byte alto do token indicando a tabela:

```c
mdtModule          = 0x00000000   mdtTypeRef      = 0x01000000
mdtTypeDef         = 0x02000000   mdtFieldDef     = 0x04000000
mdtMethodDef       = 0x06000000   mdtParamDef     = 0x08000000
mdtInterfaceImpl   = 0x09000000   mdtMemberRef    = 0x0a000000
mdtCustomAttribute = 0x0c000000   mdtPermission   = 0x0e000000
mdtSignature       = 0x11000000   mdtEvent        = 0x14000000
mdtProperty        = 0x17000000   mdtMethodImpl   = 0x19000000
mdtModuleRef       = 0x1a000000   mdtTypeSpec     = 0x1b000000
mdtAssembly        = 0x20000000   mdtAssemblyRef  = 0x23000000
mdtFile            = 0x26000000   mdtExportedType = 0x27000000
mdtManifestResource= 0x28000000   mdtNestedClass  = 0x29000000
mdtGenericParam    = 0x2a000000   mdtMethodSpec   = 0x2b000000
mdtGenericParamConstraint = 0x2c000000
mdtString          = 0x70000000   // índice no heap #US (strings literais)
mdtName            = 0x71000000
mdtBaseType        = 0x72000000   // não corresponde a tabela
```

Macros no mesmo arquivo:

```c
#define TokenFromRid(rid,tktype) ((rid) | (tktype))
#define RidFromToken(tk)         ((RID)((tk) & 0x00ffffff))
#define TypeFromToken(tk)        ((ULONG32)((tk) & 0xff000000))
#define IsNilToken(tk)           ((RidFromToken(tk)) == 0)
```

Logo, o token `0x06000012` é "MethodDef, linha (RID) 18". RID 0 é o token nulo (`mdTokenNil`, `mdTypeDefNil`...). O limite de 24 bits de RID = ~16 milhões de linhas por tabela.

Opcodes como `call`, `ldfld`, `newobj` têm operando `InlineMethod`/`InlineField`/`InlineType`/`InlineTok` (ver `opcode.def`): o operando de 4 bytes é um token.

### As tabelas físicas

A lista de tabelas do modelo mínimo está em `src/coreclr/inc/metamodelpub.h` (macro `MiniMdTable(...)`, linhas ~1650–1694):

`Module, TypeRef, TypeDef, FieldPtr, Field, MethodPtr, Method, ParamPtr, Param, InterfaceImpl, MemberRef, Constant, CustomAttribute, FieldMarshal, DeclSecurity, ClassLayout, FieldLayout, StandAloneSig, EventMap, EventPtr, Event, PropertyMap, PropertyPtr, Property, MethodSemantics, MethodImpl, ModuleRef, TypeSpec, ImplMap, FieldRVA, ENCLog, ENCMap, Assembly, AssemblyProcessor, AssemblyOS, AssemblyRef, AssemblyRefProcessor, AssemblyRefOS, File, ExportedType, ManifestResource, NestedClass, GenericParam, MethodSpec, GenericParamConstraint`.

Para o formato de debug (Portable PDB) existem tabelas adicionais 0x30–0x37 (`Document`, `MethodDebugInformation`, `LocalScope`, `LocalVariable`, `LocalConstant`, `ImportScope`, `StateMachineMethod`, `CustomDebugInformation`), com `Dummy1..3` ocupando 0x2D–0x2F — ver arquivo 09. As tabelas `*Ptr` (FieldPtr, MethodPtr, ParamPtr, EventPtr, PropertyPtr) são indireções usadas apenas em metadata "não comprimida"/editável (`#-`); `metamodel.h` tem `g_PtrTableIxs` e a lógica `fUsePointers`.

#### As quatro tabelas centrais

| Tabela | Token | Uma linha representa | Colunas principais |
|---|---|---|---|
| **TypeDef** | `0x02` | Um tipo **definido neste módulo** (classe, struct, interface, enum, delegate) | `Flags` (`CorTypeAttr`), `TypeName`, `TypeNamespace`, `Extends` (coded index), `FieldList`, `MethodList` |
| **MethodDef** | `0x06` | Um método **definido aqui** | `RVA` (corpo IL), `ImplFlags`, `Flags`, `Name`, `Signature` (blob), `ParamList` |
| **TypeRef** | `0x01` | Um tipo **definido em outro módulo/assembly** | `ResolutionScope` (AssemblyRef/ModuleRef/TypeRef pai), `TypeName`, `TypeNamespace` |
| **MemberRef** | `0x0a` | Um método ou campo **referenciado**, de outro tipo (ou de um TypeSpec) | `Class` (coded index), `Name`, `Signature` (blob) |

Observações essenciais:

- **TypeDef.FieldList / MethodList** são "índices de início": os campos/métodos de um tipo são o intervalo contíguo até a linha inicial do próximo tipo. Por isso as tabelas Field e Method são ordenadas por tipo dono. (Veja `metamodel.h`, macros `getEnd##fld##Of##tbl`, que calculam o fim do intervalo olhando a próxima linha.)
- **TypeRef** é como um módulo diz "preciso de `System.Object` do assembly X": `ResolutionScope` aponta para um `AssemblyRef` (que contém nome, versão, cultura, public key token). A resolução em tempo de execução liga TypeRef → TypeDef em outro assembly.
- **MemberRef** é a referência "por nome + assinatura" a um membro externo; o runtime resolve fazendo busca por nome e comparando a assinatura (arquivo 04). **MethodDef** é a referência direta (token → RVA).
- **Visibilidade/atributos** do tipo ficam em `TypeDef.Flags`. Em `corhdr.h`: `tdVisibilityMask = 0x7` (`tdNotPublic`=0, `tdPublic`=1, `tdNestedPublic`=2 ... `tdNestedFamORAssem`=7); `tdLayoutMask = 0x18` (auto/sequential/explicit/extended); `tdClassSemanticsMask = 0x20` (`tdClass` vs `tdInterface`).

### Coded indexes (índices codificados)

Para economizar espaço, colunas que podem apontar para várias tabelas usam *coded indexes*: alguns bits baixos escolhem a tabela, o resto é o RID. Exemplo confirmado em `src/coreclr/inc/cor.h` (`CorSigDecodeTokenType`) para referência de tipo em assinaturas:

```c
static const mdToken s_tableTokenTypes[] = {mdtTypeDef, mdtTypeRef, mdtTypeSpec, mdtBaseType};
```

Ou seja, `TypeDefOrRefOrSpecEncoded` usa 2 bits de tag: `00`=TypeDef, `01`=TypeRef, `10`=TypeSpec. Por exemplo `TypeDef.Extends` usa o mesmo esquema. A largura de uma coluna (2 ou 4 bytes) depende do **número de linhas** das tabelas possíveis e dos bits de tag — por isso `CMiniMdSchema` guarda `m_cRecs[TBL_COUNT]` (contagem de linhas por tabela) e `InitColsForTable` calcula tamanhos (`metamodel.h`, ~linhas 291 e 572).

### Heaps (streams)

A metadata física é um conjunto de *streams* dentro do arquivo: `#~` (ou `#-`) com as tabelas, `#Strings` (identificadores UTF-8), `#US` (literais de string user-defined, UTF-16, origem do `mdtString`), `#GUID`, `#Blob` (assinaturas, valores de constantes, atributos). Os índices para `#Strings`/`#Blob`/`#GUID` ocupam 2 ou 4 bytes. O addendum do repositório fixa um limite: *"the limit on #String and #Blob heap size is 2^29 (0.5 GB), that is any index to these heaps fits into 29 bits"* (Ecma-335-Augments.md, "Heap sizes"), motivado por compatibilidade com inteiros comprimidos (arquivo 04).

O Portable PDB usa o mesmo formato com stream `#Pdb` e `#~` (ver `docs/design/specs/PortablePdb-Metadata.md`), reutilizando o código de `src/coreclr/md`.

### Implementação no runtime

- `src/coreclr/md/` — biblioteca de metadata: `compiler/` (emissão — `RegMeta`), `runtime/` (leitura/`CMiniMd`), `heaps/`, `tables/`, `enc/` (Edit-and-Continue: `ENCLog`/`ENCMap`), `inc/` (`metamodel.h`, `metamodelro.h` read-only, `metamodelrw.h` read-write).
- Leitura mapeada: o runtime lê o arquivo mapeado em memória, sem desserializar; o acesso é `GetXyzRecord(rid)` (macro `MiniMdTable`).
- `src/coreclr/inc/cor.h` e `corhdr.h` — tipos públicos (tokens, flags, `IMetaDataImport`/`Emit`).
- Em código gerenciado moderno, `System.Reflection.Metadata` é o leitor/escritor (usado por Roslyn, ILCompiler: `Internal.TypeSystem.Ecma` em `src/coreclr/tools/aot/ILCompiler.TypeSystem`).

### Metadata e carregamento de tipos

Quando o runtime carrega um tipo, ele parte de um `TypeDef` e constrói a estrutura nativa (`MethodTable`) lendo campos/métodos/interfaces (`InterfaceImpl`), layout (`ClassLayout`, `FieldLayout`), genéricos (`GenericParam`, `GenericParamConstraint`). Ver `docs/design/coreclr/botr/type-loader.md` e `type-system.md`. Em R2R (arquivo 05) a metadata completa é mantida e só se *adiciona* código nativo ao lado ("the CLI file format ... is a set of database-like tables ... pay for play", `readytorun-overview.md`).

## Por quê

- **Autodescrição**: um assembly é suficiente para reflexão, ligação, verificação e ferramentas — sem headers separados.
- **Resiliência a versão**: IL referencia por nome (TypeRef/MemberRef) e não por offset; trocar a versão do assembly dependente não exige recompilar, apenas re-resolver. (`readytorun-overview.md`: "Looking carefully at the CLI file format shows that it is really 'not that bad' as a starting point... it is already public and version resilient.")
- **Compactação e mmap**: tabelas com colunas de largura variável e heaps deduplicados; acesso O(1) por RID; ordem de linhas permite busca binária em tabelas ordenadas.
- **Extensibilidade**: novas tabelas/streams podem ser acrescentadas (ex.: tabelas de Portable PDB 0x30+), e `CustomAttribute` permite anexar dados arbitrários a qualquer entidade.
- **Edição incremental**: os logs ENC (`ENCLog`, `ENCMap`) viabilizam Edit-and-Continue.

## Trade-offs

| Decisão | Benefício | Custo |
|---|---|---|
| Referência por nome (MemberRef) | Resiliência a mudança de layout/ordem | Resolução em runtime (busca por nome + comparação de assinatura) — mitigada por caches e, em R2R, por fixups |
| Tokens de 32 bits (8 bits tabela + 24 bits RID) | Operando de IL pequeno | Teto de 16M linhas por tabela |
| Coded indexes | Menos bytes por coluna | Decodificação (shift/mask), formato menos legível |
| Metadata completa preservada (inclusive em R2R) | Reflexão e carregamento sempre funcionam | Tamanho do arquivo; "Type loading from IL metadata ... prevents stripping full metadata from the image" (`readytorun-format.md`, "Future Improvements") |
| Heaps com teto de 2^29 | Bits livres para a implementação | Limita arquivos gigantescos (Augments, "Heap sizes") |
| Tabelas ordenadas por chave | Busca binária, intervalos "início-até-próximo" | Escritores precisam ordenar/remapear tokens ao salvar (`PreSaveFull` em `metamodelrw.h`) |

Em Native AOT a metadata de runtime é **substituída** por um formato próprio mais compacto, gerado só para o que é necessário (`src/coreclr/tools/aot/ILCompiler.MetadataTransform`), pois o ILC não carrega o `.dll` original — arquivo 07.

## Como aplicar na sua linguagem

1. **Separe "definições" de "referências"**: tenha uma tabela para o que o módulo define (TypeDef/MethodDef) e outra para o que ele importa por nome (TypeRef/MemberRef). Isso é o coração do ligação separada e da resiliência a versão.
2. **Use tokens `(tabela << 24) | rid`** ou equivalente. O benefício é operando de IL fixo e dispatch por tabela barato. Reserve RID 0 como nulo.
3. **Layout em intervalos**: faça "lista de campos/métodos" como índice inicial + fim implícito, evitando listas por tipo.
4. **Heaps deduplicados** de strings, blobs e GUIDs, com índice de largura variável (2 ou 4 bytes) conforme tamanho do heap.
5. **Tabela de assembly refs com identidade completa** (nome, versão, cultura, chave pública/token — arquivo 11), para ligação inequívoca.
6. **Defina o formato de assinatura (arquivo 04) antes**: MemberRef depende dele para desambiguar sobrecargas.
7. **Gancho de extensão** (equivalente a `CustomAttribute`): tabela genérica "anexar blob a (entidade, tipo)". Isso permitiu ao .NET evoluir sem novas tabelas para cada recurso.
8. **Considere um formato estrito para consumo (mmap) e outro para emissão**: o .NET tem `CMiniMdRW` (editável) e `CMiniMd` (somente leitura); o escritor ordena, remapeia tokens e comprime ao salvar.
9. **Metadata para debug em arquivo separado** (arquivo 09) com o mesmo formato de tabelas, reutilizando o leitor.
10. Se for ter AOT, planeje desde cedo **um segundo formato de metadata reduzido** (apenas o que a reflexão exige), já que a metadata completa é cara e vaza nomes.

## Referências

- `src/coreclr/inc/corhdr.h` — `CorTokenType`, `RidFromToken`/`TypeFromToken`, `CorTypeAttr`, `CorAssemblyFlags`.
- `src/coreclr/inc/cor.h` — `CorSigDecodeTokenType`, `CorSigUncompressToken`, `CorSigUncompressData`.
- `src/coreclr/inc/metamodelpub.h` — lista de tabelas (`MiniMdTable`).
- `src/coreclr/md/inc/metamodel.h`, `metamodelro.h`, `metamodelrw.h` — modelo de tabelas, esquema, colunas.
- `src/coreclr/md/` — implementação (`compiler`, `runtime`, `heaps`, `enc`).
- `docs/design/specs/Ecma-335-Augments.md` — "Heap sizes", "Metadata logical format", "Metadata merging".
- `docs/design/specs/PortablePdb-Metadata.md` — tabelas 0x30–0x37.
- `docs/design/coreclr/botr/type-loader.md`, `type-system.md`, `method-descriptor.md` — uso das tabelas no runtime.
- ECMA-335 Partição II §22 (tabelas de metadata), §23 (formatos de blobs/assinaturas), §24 (layout físico: streams `#~`, `#Strings`, `#US`, `#GUID`, `#Blob`).

## Apêndice A — Resolução de um `call` passo a passo

Considere uma instrução `call 0x0A00001F` em algum método:

1. `TypeFromToken(0x0A00001F) = 0x0A000000 = mdtMemberRef`; `RidFromToken = 0x1F = 31`. Lê-se a linha 31 de `MemberRef`.
2. A coluna `Class` é um *coded index* que pode apontar para `TypeRef`, `ModuleRef`, `MethodDef`, `TypeDef` ou `TypeSpec`. Suponha que seja um `TypeRef` (ex.: `System.Console`).
3. O `TypeRef` tem `ResolutionScope` → `AssemblyRef` (ex.: `System.Console, Version=..., PublicKeyToken=...`) — identidade no arquivo 11.
4. O binder resolve o `AssemblyRef` para um assembly carregado (`src/coreclr/binder/`), onde se procura o `TypeDef` com o mesmo namespace/nome.
5. Nesse `TypeDef`, o runtime itera `MethodList` procurando um `MethodDef` com o mesmo **nome** e **assinatura** (blob de `MemberRef.Signature`, arquivo 04). Achado, o token resolvido vira um `MethodDesc` interno (ver `docs/design/coreclr/botr/method-descriptor.md`) com cache — assim os custos de resolução são pagos uma vez.

Para um método do próprio módulo, o token é `mdtMethodDef (0x06...)` e o passo 2–5 é dispensado: `MethodDef.RVA` já aponta para o corpo.

Em R2R, essa resolução vira **fixup**: o slot do call site é preenchido sob demanda com uma assinatura `READYTORUN_FIXUP_MethodEntry` / `MethodEntry_DefToken` / `MethodEntry_RefToken` (arquivo 05), que referencia o mesmo token de metadata.

## Apêndice B — Esboço de um formato de metadata mínimo para sua linguagem

Inspirado nas tabelas centrais (não é a spec do .NET):

| Tabela | Colunas mínimas | Observação |
|---|---|---|
| `Module` | nome, GUID de versão (MVID) | identifica a compilação |
| `TypeDef` | flags, nome, namespace, `extends`, `fieldList`, `methodList` | intervalos contíguos |
| `Field` | flags, nome, assinatura | |
| `Method` | RVA do corpo, flags, nome, assinatura, `paramList` | |
| `TypeRef` / `MemberRef` | escopo, nome, (assinatura) | importações por nome |
| `AssemblyRef` | nome, versão, cultura, token de chave | identidade (arquivo 11) |
| `CustomAttribute` | pai (coded index), tipo do atributo, valor (blob) | extensão |
| `GenericParam` / `MethodSpec` | número, flags, nome / método + instanciação | arquivo 08 |

Regras práticas:

- Cada tabela com linhas ordenadas por chave de ligação pai→filho precisa ser ordenada ao salvar; o escritor do .NET (`CMiniMdRW`) ordena e **remapeia tokens** na etapa de salvar (`PreSaveFull`).
- Escolha o tamanho das colunas no momento de salvar com base na **contagem de linhas** (`m_cRecs`): colunas de 2 bytes quando todas as tabelas-alvo cabem em 16 bits menos os bits de tag.
- Reserve tabelas "vagas" no ID-space (o .NET tem `Dummy1..3` entre 0x2D e 0x2F e salta para 0x30 no formato de debug) para evoluir sem renumerar.
- Para edição incremental, mantenha log de mudanças (`ENCLog`/`ENCMap`) separado das tabelas principais.

## Apêndice C — Armadilhas observadas

1. **Referências cruzadas sem cuidado com nome+assinatura**: duas sobrecargas com mesma lista de parâmetros mas retornos distintos precisam de assinaturas diferentes (o retorno faz parte do blob).
2. **Teto de heap** (2^29): fixe-o na sua spec desde o dia 1 (o .NET precisou explicitar depois — `Ecma-335-Augments.md`, "Heap sizes").
3. **Duas representações para a mesma entidade** (ex.: `TypeSpec` direto sob `CLASS`) geram bugs de identidade; imponha forma canônica.
4. **Metadata de reflexão ≠ metadata de execução**: se você pretende ter AOT, não assuma que o runtime sempre terá acesso às tabelas originais.
