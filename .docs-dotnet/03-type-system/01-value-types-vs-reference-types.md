# Value types vs reference types (representação no runtime)

> Convenção deste documento: afirmações marcadas com caminho de arquivo foram lidas no repositório
> `dotnet/runtime`. Trechos marcados como "(spec/conhecimento geral)" vêm do ECMA-335/C# e não foram
> re-verificados em código aqui.

## Mecanismo no .NET

### 1. A divisão fundamental está no próprio MethodTable

Todo tipo "comum" (classes, structs, enums, primitivos, genéricos abertos/fechados) é representado
por um `MethodTable`; tipos "sintéticos" (byref, ponteiro, function pointer, variável genérica) são
representados por um `TypeDesc`. O identificador universal de tipo é o `TypeHandle`, que
encapsula um ponteiro para um dos dois; o bit 1 do ponteiro (`ptr | 2`) distingue `TypeDesc` de
`MethodTable` (`docs/design/coreclr/botr/type-loader.md`, seção do `TypeHandle`).

A "categoria" do tipo é codificada em bits dos flags do `MethodTable`
(`src/coreclr/vm/methodtable.h`, ~linhas 3810-3830):

```
enum_flag_Category_Mask          = 0x000F0000
enum_flag_Category_Class         = 0x00000000
enum_flag_Category_ValueType     = 0x00040000
enum_flag_Category_Nullable      = 0x00050000  // subcategoria de ValueType
enum_flag_Category_Primitive     = 0x00060000  // subcategoria de ValueType; usado p/ enums
enum_flag_Category_TruePrimitive = 0x00070000  // Int32 etc.
enum_flag_Category_Array         = 0x00080000
```

Ou seja: "value type" é uma categoria de primeira classe do tipo, com subcategorias para
`Nullable<T>`, enums e primitivos verdadeiros. Interfaces têm sua própria categoria
(`enum_flag_Category_Interface`, `methodtable.h` ~linha 1431).

### 2. Layout em memória

- **Referência (classe)**: a "referência" é o endereço do objeto; o primeiro campo, no offset 0,
  é o ponteiro para o `MethodTable` (`type-loader.md`, seção 2.2: "All instances of the type
  contain a pointer to the `MethodTable` as their first field at offset 0, i.e. at the address
  seen as the reference value"). Há também um cabeçalho (object header / sync block) antes desse
  endereço (detalhe do GC/sync block, não detalhado nos arquivos lidos aqui).
- **Value type não-boxed**: apenas os campos, sem ponteiro de MethodTable e sem cabeçalho. O tipo
  é conhecido estaticamente pelo código (JIT/assinatura). Isso é visível no código de unbox:
  `CastHelpers.Unbox` devolve `ref obj.GetRawData()` — o dado do struct começa logo após o
  ponteiro de MethodTable do objeto *boxed* (`src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs`, ~linha 400).
- **Value type boxed**: um objeto de heap cujo MethodTable é o do próprio struct e cujo
  corpo são os campos (ver `06-boxing-unboxing.md`).

### 3. Flags relevantes para o GC e para o sistema de tipos

O `MethodTable` carrega, entre outros, `enum_flag_ContainsGCPointers` (0x01000000), usado
tanto para alocação quanto para copiar structs com write barrier: em `CastHelpers.Box`, se
`typeMT->ContainsGCPointers` usa `Buffer.BulkMoveWithWriteBarrier`, senão `SpanHelpers.Memmove`.
Ou seja, um struct que contém referências precisa de tratamento especial de cópia; um struct
"blittable/sem GC" copia por memcpy.

`enum_flag_IsByRefLike` (0x1000) marca `ref struct` (`methodtable.h` ~3775), e
`enum_flag_HasVariance` marca instanciações com parâmetros co/contravariantes (~3756).

### 4. `ValueType` e `Object` como bases

Value types derivam (no modelo de tipos) de `System.ValueType`, que é uma classe abstrata
(`src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`, `public abstract partial class ValueType`).
Ela fornece `Equals`/`GetHashCode` genéricos. Há um caminho rápido: se
`CanCompareBitsOrUseFastGetHashCode(MethodTable*)` for verdadeiro (o struct não tem padding
nem campos que exijam comparação especial), a igualdade é uma comparação de bits; caso
contrário usa-se reflexão campo a campo (`ValueType.cs`, linhas ~41-115). O resultado dessa
análise é cacheado no `AuxiliaryData` do MethodTable (`HasCheckedCanCompareBitsOrUseFastGetHashCode`).

Implicação: o valor de um struct "herda" métodos virtuais de `Object`/`ValueType`, mas a
chamada a esses métodos sobre um struct não-boxed é feita via `constrained. callvirt`, para
evitar boxing quando o struct sobrescreve o método (spec: ECMA-335 III.2.1; o trecho
`Ecma-335-Augments.md` linha ~434 confirma que, para default interface methods, o boxing
implícito se torna observável).

### 5. Semântica de cópia e identidade

(spec/conhecimento geral) Atribuir/passar um value type copia os bytes; atribuir uma referência
copia o ponteiro. Igualdade de referência (`ceq`) compara endereços; value types não têm
identidade. Consequências concretas vistas no código:

- `Nullable<T>` é um struct com dois campos (`bool hasValue; T value;`) — `src/libraries/System.Private.CoreLib/src/System/Nullable.cs`.
- `Span<T>` é `readonly ref struct` com `ref T _reference` e `int _length`
  (`src/libraries/System.Private.CoreLib/src/System/Span.cs`, linhas ~28-33): só é possível
  porque value types podem conter *managed pointers* (byref), o que classes não podem.

### 6. Impacto em genéricos

A divisão ref/value determina a estratégia de compilação de genéricos: instanciações sobre
tipos de referência compartilham o mesmo `EEClass` e o mesmo código (todos têm tamanho de
ponteiro); instanciações sobre value types ganham `EEClass` e código próprios
(`type-loader.md` seção 2.2.2; `docs/design/coreclr/botr/shared-generics.md`). Ver `02` e `03`.

## Por quê

- **Densidade e localidade**: structs inline em arrays/objetos evitam um ponteiro e um header
  por elemento; array de `int` é um bloco contíguo. (Inferência razoável a partir do layout; a
  motivação histórica não está documentada nos arquivos lidos.)
- **Custo de GC**: struct sem referências não tem nada para o GC marcar (`ContainsGCPointers`
  falso). Isso é diretamente observável no flag.
- **Semântica de valor para tipos pequenos e imutáveis** (inteiros, `DateTime`, `Nullable<T>`,
  tuplas): copiar é barato e evita alocação.
- **Separação em nível de metadado**: como o tipo é parte do metadado (`TypeDef` estende
  `ValueType` ou não), o carregador de tipos decide a categoria uma vez, na construção do
  MethodTable (`MethodTableBuilder`, `src/coreclr/vm/methodtablebuilder.cpp`).

## Trade-offs

| Aspecto | Value type | Reference type |
|---|---|---|
| Alocação | stack/inline, sem GC (se não boxed) | heap, GC |
| Cópia | O(tamanho) a cada passagem | O(1) (ponteiro) |
| Identidade | nenhuma | endereço |
| Polimorfismo | só via interface/boxing ou genéricos | virtual dispatch direto |
| Mutabilidade | cópias divergem (armadilha clássica) | compartilhada |
| Genéricos | uma instanciação de código por tipo (inchaço) | código compartilhado |
| `null` | não existe (precisa `Nullable<T>`) | nativo |

Custos adicionais:
- Structs grandes são caros de copiar; o runtime não os promove automaticamente a referência.
- Tratar um struct como `object`/interface exige boxing (alocação + cópia) — ver `06`.
- Duas "famílias" de tipos complicam a linguagem, o JIT, o GC (ref-maps de structs com
  ponteiros), as regras de genéricos e de variância (value types não participam de variância,
  ver `05`).
- Há restrições sobre onde um struct pode viver quando contém byref (`ref struct`; ver `08`).

## Como aplicar na sua linguagem

1. **Decida cedo se terá duas famílias.** Se sim, torne a categoria um campo explícito da
   representação de tipo (como `Category_ValueType` vs `Class`), não um efeito colateral.
2. **Defina o contrato de representação**: reference = ponteiro para objeto com header de tipo;
   value = campos inline sem header. Documente que o tipo de um value é *estático*.
3. **Guarde no descritor de tipo, no mínimo**: tamanho, alinhamento, "contém ponteiros GC?",
   "é blittable?", "pode ser comparado por bits?" — o .NET usa esses bits para escolher memcpy
   vs cópia com write barrier e igualdade rápida.
4. **Prefira uma alternativa mais simples se não precisar de interop de baixo nível**: um
   modelo "tudo é referência + otimizador de escape analysis" evita a dualidade, ao custo de
   previsibilidade. A escolha .NET dá controle ao programador e exige que ele entenda boxing.
5. **Projete `null` junto**: se value types não aceitam null, forneça um tipo opcional
   (`Nullable<T>`-like) e decida se ele é especial para o runtime (ver `09`).
6. **Defina operações universais** (`equals`, `hash`, `toString`) com fast-path para structs
   sem padding, como `ValueType.Equals`.
7. **Impeça value types recursivos** (um struct não pode conter a si mesmo por valor) — o
   carregador precisa conseguir calcular o tamanho.

## Referências

- `docs/design/coreclr/botr/type-loader.md` (TypeHandle, MethodTable/EEClass, sharing)
- `docs/design/coreclr/botr/shared-generics.md`
- `src/coreclr/vm/methodtable.h` (flags de categoria, `ContainsGCPointers`, `IsByRefLike`, `HasVariance`)
- `src/coreclr/vm/methodtablebuilder.cpp`
- `src/coreclr/vm/typedesc.h` (`TypeDesc`, `TypeVarTypeDesc`, `FnPtrTypeDesc`)
- `src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs`
- `src/libraries/System.Private.CoreLib/src/System/Nullable.cs`
- `src/libraries/System.Private.CoreLib/src/System/Span.cs`
- `docs/design/specs/Ecma-335-Augments.md`
