# Assinaturas (signatures) e encoding de tipos em metadata

## Mecanismo no .NET

Quase tudo que descreve "forma" em metadata — o tipo de um campo, os parâmetros de um método, os tipos de variáveis locais, uma instanciação genérica — é guardado como uma **assinatura**: uma sequência de bytes no heap `#Blob`, referenciada por índice (colunas `Signature` de `Field`, `MethodDef`, `MemberRef`, `StandAloneSig`, `Property`, `TypeSpec`, `MethodSpec`). A gramática está em ECMA-335 Partição II §23.2. O runtime lê com `SigParser` (`src/coreclr/inc/sigparser.h`) e escreve com `SigBuilder` (`src/coreclr/inc/sigbuilder.h`).

### Inteiros comprimidos

Todos os números em assinaturas (contagem de parâmetros, rank, índices de variáveis genéricas, tokens) usam a compressão de ECMA-335 II.23.2. O decodificador real está em `src/coreclr/inc/cor.h` (`CorSigUncompressData` / `CorSigUncompressBigData`):

| Primeiro byte | Bytes | Faixa |
|---|---|---|
| `0xxxxxxx` | 1 | 0 a 0x7F |
| `10xxxxxx` + 1 byte | 2 | até 0x3FFF |
| `110xxxxx` + 3 bytes | 4 | até 0x1FFFFFFF (2^29 − 1) |

No código: `if ((*pData & 0x80) == 0) return *pData++;` ... `(*pData & 0xC0) == 0x80` → `((b & 0x3f) << 8) | next` ... senão `110?` → 4 bytes big-endian com `(b & 0x1f) << 24`. O teto 2^29 é o motivo do limite dos heaps (`Ecma-335-Augments.md`, "Heap sizes").

### Tipos de elemento: `CorElementType`

`src/coreclr/inc/corhdr.h` (linhas ~875–926) define os bytes que compõem um tipo:

| Código | Nome | Sintaxe / observação |
|---|---|---|
| 0x01 | `VOID` | só em retorno / alvo de ponteiro |
| 0x02..0x0D | `BOOLEAN, CHAR, I1, U1, I2, U2, I4, U4, I8, U8, R4, R8` | primitivos |
| 0x0E | `STRING` | |
| 0x0F | `PTR` | `PTR <type>` |
| 0x10 | `BYREF` | `BYREF <type>` |
| 0x11 | `VALUETYPE` | `VALUETYPE <class token>` |
| 0x12 | `CLASS` | `CLASS <class token>` |
| 0x13 | `VAR` | variável de tipo da classe: `VAR <number>` |
| 0x14 | `ARRAY` | `MDARRAY <type> <rank> <bcount> <bound1>... <lbcount> <lb1>...` |
| 0x15 | `GENERICINST` | `GENERICINST <generic type> <argCnt> <arg1>...<argn>` |
| 0x16 | `TYPEDBYREF` | sem argumentos |
| 0x18 / 0x19 | `I` / `U` | `native int` / `native unsigned int` |
| 0x1B | `FNPTR` | `FNPTR <assinatura completa com convenção>` |
| 0x1C | `OBJECT` | atalho para `System.Object` |
| 0x1D | `SZARRAY` | atalho: vetor unidimensional de base zero |
| 0x1E | `MVAR` | variável de tipo do **método**: `MVAR <number>` |
| 0x1F / 0x20 | `CMOD_REQD` / `CMOD_OPT` | modificador custom obrigatório/opcional `<TypeDefOrRef>` |
| 0x21 / 0x22 | `INTERNAL` / `CMOD_INTERNAL` | só uso interno do runtime (typehandle direto) |
| 0x23 | `MAX` | primeiro inválido |
| 0x40 | `MODIFIER` | bit que marca modificadores |
| 0x41 | `SENTINEL` | marcador de varargs (`0x01 \| MODIFIER`) |
| 0x45 | `PINNED` | local fixado (`0x05 \| MODIFIER`) |

(Os valores 0x41 e 0x45 resultam da aritmética `ELEMENT_TYPE_SENTINEL = 0x01 | ELEMENT_TYPE_MODIFIER` etc. do próprio header.)

Notas:
- `ELEMENT_TYPE_VALUETYPE`/`CLASS` informam se o tipo é valor ou referência **dentro da assinatura**; o runtime precisa dessa distinção para calcular layout de chamada sem carregar o tipo (o header avisa: "Please use ELEMENT_TYPE_VALUETYPE. ELEMENT_TYPE_VALUECLASS is deprecated").
- `ELEMENT_TYPE_INTERNAL` (0x21) e `CMOD_INTERNAL` (0x22) são marcados `[cDAC]` no header: o contrato do depurador de dados depende de seus valores; não aparecem em arquivos no disco, só em assinaturas sintetizadas.

### Referência a tipos dentro da assinatura

`TypeDefOrRefOrSpecEncoded`: um inteiro comprimido cujos 2 bits baixos são o tag (`TypeDef`=0, `TypeRef`=1, `TypeSpec`=2) e o resto é o RID. Em código: `CorSigDecodeTokenType` com `{mdtTypeDef, mdtTypeRef, mdtTypeSpec, mdtBaseType}` (`cor.h`). O addendum do repo (`Ecma-335-Augments.md`, seção "Signatures" #1) esclarece que `CLASS`/`VALUETYPE` **não podem** apontar para `TypeSpec` na prática (evita ciclos e dupla representação), propondo gramática `TypeDefOrRefEncoded` e `Type+` (instanciação genérica com pelo menos 1 argumento).

### Convenção de chamada (primeiro byte de uma assinatura de método)

De `corhdr.h` (~linhas 975–995):

```
IMAGE_CEE_CS_CALLCONV_DEFAULT      = 0x0
IMAGE_CEE_CS_CALLCONV_C / STDCALL / THISCALL / FASTCALL   (nativas, p/invoke)
IMAGE_CEE_CS_CALLCONV_VARARG       = 0x5
IMAGE_CEE_CS_CALLCONV_FIELD        = 0x6   // assinatura de campo
IMAGE_CEE_CS_CALLCONV_LOCAL_SIG    = 0x7   // assinatura de locais
IMAGE_CEE_CS_CALLCONV_PROPERTY     = 0x8
IMAGE_CEE_CS_CALLCONV_UNMANAGED    = 0x9   // convenção nativa codificada como modopts
IMAGE_CEE_CS_CALLCONV_GENERICINST  = 0xa   // instanciação de método genérico
IMAGE_CEE_CS_CALLCONV_NATIVEVARARG = 0xb
IMAGE_CEE_CS_CALLCONV_ASYNC        = 0xc   // usado p/ calli em IL stubs
IMAGE_CEE_CS_CALLCONV_MASK         = 0x0f  // 4 bits baixos
IMAGE_CEE_CS_CALLCONV_GENERIC      = 0x10  // método genérico: vem antes da contagem de params
IMAGE_CEE_CS_CALLCONV_HASTHIS      = 0x20
IMAGE_CEE_CS_CALLCONV_EXPLICITTHIS = 0x40
```

Gramática simplificada de uma assinatura de método:

```
MethodDefSig ::= [HASTHIS] [EXPLICITTHIS] (DEFAULT | VARARG | GENERIC GenParamCount)
                 ParamCount RetType Param*
Param/RetType ::= CustomMod* (BYREF? Type | TYPEDBYREF | VOID)
```

Exemplo: o método de instância `string Foo(int x, List<string> y)` seria (conceitualmente):

```
20            ; DEFAULT | HASTHIS
02            ; 2 parâmetros
0E            ; retorno: STRING
08            ; param 1: I4
15 12 <tk>    ; param 2: GENERICINST CLASS <TypeRef List`1>
01            ;   1 argumento genérico
0E            ;   STRING
```

(`<tk>` é o `TypeDefOrRef` comprimido.) Já `void M<T>(T[] a)` seria `30 01 01 01 1D 1E 00`: `0x30` = `GENERIC|HASTHIS` (para método estático seria `0x10`), 1 parâmetro de tipo, 1 parâmetro, retorno `VOID`, parâmetro `SZARRAY MVAR 0`.

### Locais, campos, propriedades, MethodSpec

- **Field**: `FIELD(0x6) CustomMod* Type`.
- **LocalVarSig** (em `StandAloneSig`, apontada pelo header do corpo de método): `LOCAL_SIG(0x7) Count (CustomMod* (TYPEDBYREF | [PINNED] Type))+`.
- **TypeSpec**: blob com um `Type` completo (usado para `List<int>`, arrays, ponteiros) — é o que `call`/`newobj` referenciam quando o alvo é instanciado.
- **MethodSpec**: `GENERICINST(0xa) ArgCount Type+` para instanciar um método genérico.

### Modificadores custom (modreq/modopt)

`CMOD_REQD`/`CMOD_OPT` anexam um tipo "marcador" a um tipo (por exemplo `volatile`, `in`, convenção de chamada em `UNMANAGED`). O addendum documenta casos delicados: `(CMOD_OPT|CMOD_REQ) <TypeSpec>` é inválido (#2), modificadores podem aparecer em mais lugares do que a spec diz (#3), `BYREF` pode vir antes de modificadores (#4) e `TypeSpec` pode codificar mais do que a spec afirma (#5).

### Assinaturas além da ECMA: R2R

`docs/design/coreclr/botr/readytorun-format.md` mostra que a ECMA não basta para código nativo: *"MethodSpec signatures defined by ECMA-335 are not rich enough to describe method flavors referenced by native code."* O R2R estende com assinaturas de fixup que começam por um byte "kind" (`READYTORUN_FIXUP_*`) e flags de método: `UnboxingStub=0x01`, `InstantiatingStub=0x02`, `MethodInstantiation=0x04`, `SlotInsteadOfToken=0x08`, `MemberRefToken=0x10`, `Constrained=0x20`, `OwnerType=0x40`, `UpdateContext=0x80`, `AsyncVariant=0x100`. Os fixups reaproveitam o encoding de tipo da ECMA (typespec) e adicionam `ModuleOverride=0x80`.

### Pretty printing e utilitários

`src/coreclr/inc/prettyprintsig.h` imprime assinaturas legíveis; `SigParser::SkipExactlyOne()` pula um tipo completo (necessário porque tipos têm comprimento variável).

## Por quê

- **Compactação**: um tipo comum (`int`, `string`) custa 1 byte; inteiros comprimidos mantêm contagens pequenas em 1 byte.
- **Parsing sem contexto**: cada tipo é prefixado por seu `ElementType`, o que permite parser de uma passagem, sem tabela de símbolos.
- **Identidade estrutural**: dois métodos são "o mesmo" para ligação quando nome + assinatura (bytes ou estrutura) coincidem; é a base da resolução de `MemberRef` (arquivo 02).
- **Evolução**: o bit-prefix de convenção de chamada comporta flags (`HASTHIS`, `GENERIC`) e variantes (`FIELD`, `LOCAL_SIG`, `PROPERTY`) no mesmo espaço.
- **Layout de chamada sem carregar tipos**: distinguir `CLASS` de `VALUETYPE` na própria assinatura permite compilar chamadas (ABI) antes de resolver o tipo — crucial no R2R/AOT.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Blob binário compacto | Menor tamanho, mmap | Ilegível; precisa de parser e printer |
| Tipos referenciados por token no módulo | Menor | Assinatura só faz sentido no contexto do módulo que a contém (daí `UpdateContext`/`ModuleOverride` no R2R) |
| Atalhos (`OBJECT`, `SZARRAY`) | Compacto | Duas representações possíveis para a mesma coisa (por isso o Augments propõe forma canônica) |
| Genéricos por índice posicional (`VAR n`, `MVAR n`) | Independente de nomes | Precisa de `GenericParam` para ler nomes/restrições |
| Gramática "flexível" (modifiers em vários lugares) | Extensibilidade | A spec ficou inconsistente com as implementações; o repo mantém um addendum |
| Convenção de chamada em 4 bits | Barato | Espaço limitado; `UNMANAGED` usa modopts para combinações novas |

## Como aplicar na sua linguagem

1. **Defina uma gramática de tipos prefixada** (um byte de "kind" seguido de parâmetros) — é trivial de decodificar e estender.
2. **Use inteiros de largura variável** com prefixo de bits (1/2/4 bytes). Documente o teto (2^29 no .NET) e use esse mesmo teto para limites de heap.
3. **Distinga valor × referência na assinatura** se sua linguagem tem structs; evita carregar tipos para decidir o ABI.
4. **Codifique parâmetros genéricos por posição** (`VAR n`/`MVAR n`) e separe "contagem de genéricos" da "contagem de parâmetros" (flag `GENERIC` + contador antes da lista).
5. **Mantenha uma forma canônica**: se tem atalho (`SZARRAY`) e forma geral (`ARRAY`), regule qual usar; evita duplicidade de identidade (lição do Augments #1).
6. **Previna ciclos**: o runtime proíbe `TypeSpec` direto sob `CLASS`/`VALUETYPE` para impedir recursão na assinatura; considere regra equivalente.
7. **Modificadores custom** (modreq/modopt) são uma boa válvula de extensão: permitem que a linguagem codifique semântica extra sem mudar o formato; `modreq` força o consumidor a entender, `modopt` pode ser ignorado.
8. **Convenções de chamada como flags + enum** (this, genérico, vararg) no primeiro byte.
9. **Se for fazer AOT/R2R**, planeje assinaturas **mais ricas** que as de IL (variantes: unboxing stub, instantiating stub, slot em vez de token) — o R2R precisou criar `READYTORUN_METHOD_SIG_*`.
10. **Ferramentas**: forneça desde o início um *skipper* (`SkipExactlyOne`) e um *pretty-printer* de assinaturas; bugs de metadata se resolvem lendo assinaturas.

## Referências

- `src/coreclr/inc/corhdr.h` — `CorElementType` (linhas ~875–926), `IMAGE_CEE_CS_CALLCONV_*` (~975–995), `SERIALIZATION_TYPE_*` (para atributos custom).
- `src/coreclr/inc/cor.h` — `CorSigUncompressData`, `CorSigUncompressBigData`, `CorSigDecodeTokenType`, `CorSigUncompressToken`.
- `src/coreclr/inc/sigparser.h`, `sigbuilder.h`, `prettyprintsig.h`.
- `docs/design/specs/Ecma-335-Augments.md` — seção "Signatures" (#1–#5), "Heap sizes".
- `docs/design/coreclr/botr/readytorun-format.md` — "Method Signatures", "Field Signatures", `READYTORUN_FIXUP_*`.
- ECMA-335 Partição II §23.1 (tipos de blobs), §23.2 (assinaturas, inteiros comprimidos, `TypeDefOrRefOrSpecEncoded`), Partição I §8 (sistema de tipos).
