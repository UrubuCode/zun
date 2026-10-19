# Structs, ref structs e readonly struct

## Mecanismo no .NET

### 1. Struct "normal"

Um `struct` é um value type (categoria `ValueType` no `MethodTable`, ver `01`). Tem layout próprio
(ver `10`), é copiado por valor e não tem header de objeto quando não está boxed.

### 2. `ref struct` = tipo ByRefLike

No runtime, "ref struct" é a propriedade **ByRefLike**: flag `enum_flag_IsByRefLike`
(`src/coreclr/vm/methodtable.h`, ~3775), setado em `MethodTableBuilder` quando o tipo possui o
atributo `IsByRefLikeAttribute` (`methodtablebuilder.cpp` ~1401-1406:
`WellKnownAttribute::IsByRefLike` → `bmtFP.fIsByRefLikeType = true` → `pMT->SetIsByRefLike()`
~1801). O atributo está em
`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IsByRefLikeAttribute.cs`.

Por que o runtime precisa disso: um ByRefLike pode conter **managed pointers** (`ref T`), que só
podem existir na stack. `Span<T>` é o exemplo canônico:

```csharp
// src/libraries/System.Private.CoreLib/src/System/Span.cs
public readonly ref struct Span<T>
{
    internal readonly ref T _reference;
    private readonly int _length;
```

### 3. Regras impostas pelo *carregador de tipos*

Em `methodtablebuilder.cpp` (~4210-4222 e 4343-4350):
- Campo **estático** de tipo byref/ByRefLike → erro de carga
  (`IDS_CLASSLOAD_BYREF_OR_BYREFLIKE_STATICFIELD`, "Byref-like types cannot be used for static
  fields").
- Campo **de instância** byref ou de tipo ByRefLike em tipo que *não* é ByRefLike → erro de carga
  (`IDS_CLASSLOAD_BYREF_OR_BYREFLIKE_INSTANCEFIELD`).
- Portanto: ByRefLike só pode ser campo de outro ByRefLike, e nunca de classe, nunca estático.

`CheckValueClassLayout` e confiança de campos em layout explícito também tratam ByRefLike
(`methodtablebuilder.cpp` ~9122).

### 4. Regras impostas pelas instruções IL (genéricos)

`docs/design/features/byreflike-generics.md` define o comportamento quando `T` é ByRefLike
(permitido com a constraint `allows ref struct`, flag `gpAcceptByRefLike = 0x0020`):
- `box` → `InvalidProgramException`.
- `stsfld`/`ldsfld` em campo cujo tipo é `T` → `TypeLoadException`.
- `newarr`/`stelem`/`ldelem`/`ldelema` → `TypeLoadException`: arrays não contêm ByRefLike.
- `throw`, `unbox`, `unbox.any`, `isinst`, `castclass` já falham por serem sobre objetos.
- Se `T` é ByRefLike e é campo, o tipo envolvente tem que ser ByRefLike.
- Exemplos de uso: `Action<Span<char>>`, `string.Create<TState>(..., SpanAction<char,TState>)`.

Logo, uma `ref struct` **não pode**: ser boxed, ser campo de classe, ser capturada por lambda/
async state machine (essas duas são regras de linguagem do C#, conhecimento geral), ser elemento de
array, ou ser argumento genérico comum.

### 5. `readonly struct` e `IsReadOnlyAttribute`

`readonly struct` é uma garantia de **linguagem/metadado**: o compilador marca o tipo com
`System.Runtime.CompilerServices.IsReadOnlyAttribute`
(`.../CompilerServices/IsReadOnlyAttribute.cs`) e todos os campos `initonly`. O objetivo é evitar
*cópias defensivas*: ao chamar um método de instância em um struct acessado por `in`/`readonly ref`
o compilador copia o struct (para proteger contra mutação) a menos que o struct seja readonly
(conhecimento geral de C#; o runtime em si não impõe — o atributo é lido pelos compiladores).
Exemplos no CoreLib: `Span<T>` é `readonly ref struct`; `Nullable<T>` marca métodos `readonly`
individualmente (`public readonly bool HasValue`, ver `Nullable.cs`).

### 6. Outros atributos relacionados

- `IsUnmanagedAttribute` — constraint `unmanaged` (structs sem referências gerenciadas) em
  `.../CompilerServices/IsUnmanagedAttribute.cs`.
- `InlineArrayAttribute` — struct com um campo que o runtime replica `Length` vezes
  (`docs/design/features/InlineArrayAttribute.md`; `methodtablebuilder.cpp` ~1716-1723
  `SetIsInlineArray`). Garante layout idêntico a elementos de array para permitir `Span` seguro
  sobre buffers inline.
- `Unsafe`, `UnsafeValueTypeAttribute`: fixed buffers.

### 7. Tipos ByRefLike no GC

Como vivem só na stack, o JIT produz mapas de GC para suas referências interiores (byref). Para
structs com `ref` field o `ContainsGCPointers` não se aplica no sentido de heap objects; o
runtime trata byrefs como "interior pointers" (conhecimento geral do GC; não detalhado nos
arquivos lidos).

## Por quê

- **Segurança de memória sem GC overhead**: `Span<T>` permite fatias de memória (heap, stack,
  nativa) sem alocar; para isso precisa ser impossível que escape para o heap (onde sobreviveria à
  memória da stack que referencia).
- **Garantia estrutural, verificada na carga**: o runtime recusa tipos inválidos em vez de confiar
  só no compilador.
- **`readonly`**: permitir passagem por referência (`in`) barata sem cópia defensiva.
- **Genéricos sobre Span** (`byreflike-generics.md`): para evitar tipos de delegate/ API duplicados.

## Trade-offs

| Aspecto | Impacto |
|---|---|
| ref struct não pode ser boxed/ campo de classe/ array / estático | Muitas APIs precisam de overloads separados ou de `allows ref struct` |
| Genéricos precisam de opt-in (`AcceptByRefLike`) | APIs antigas continuam compiladas, mas toda IL com `box`, `newarr` etc. sobre T precisa ser revalidada |
| Retrocompatibilidade | Mudar um `T` existente para aceitar ByRefLike pode invalidar APIs (o doc propõe um atributo de supressão de checagem) |
| `readonly struct` | Perde mutabilidade; ótimo para dados imutáveis, ruim para estado |
| Structs grandes | Cópia cara; `in` ajuda só se readonly |

Custo de evolução: a doc `byreflike-generics.md` lista impacto em ILDasm/ILAsm, Reflection.Metadata,
Reflection.Emit, Cecil, ILLink, F# e C++/CLI.

## Como aplicar na sua linguagem

1. **Se oferecer slices/views sem alocação, precisa de um tipo "só stack"** (ByRefLike) e das
   regras de escape. Modele isso como um flag no tipo, verificado no carregamento (campos, estáticos,
   arrays) *e* no compilador.
2. **Escreva a tabela de operações proibidas** (como a de `byreflike-generics.md`): box, array,
   estático, captura, genérico. Dê um erro por operação.
3. **Prefira opt-in nos genéricos** (`allows ref struct`) para evitar quebra de código existente.
4. **Separe imutabilidade de categoria**: `readonly` é ortogonal. Permitir que o compilador
   dispense cópias defensivas é o ganho principal.
5. **Se tiver buffers inline**, ofereça algo como `InlineArray`: um construto que o runtime
   expande com layout igual a um array para permitir `Span` seguro.
6. **Defina o que significa `unmanaged`** (sem referências gerenciadas) — base para interop e para
   `memcpy` seguro.
7. **Alternativa simples**: se sua linguagem tem borrow checker/regions, `ref struct` fica
   desnecessário (o tempo de vida é rastreado pelo tipo).

### Exemplo: o que o carregador rejeita

```csharp
ref struct R { Span<int> s; }        // OK: ByRefLike dentro de ByRefLike
class C { Span<int> s; }             // TypeLoadException: IDS_CLASSLOAD_BYREF_OR_BYREFLIKE_INSTANCEFIELD
struct S { Span<int> s; }            // idem (S não é ByRefLike)
ref struct T { static Span<int> x; } // TypeLoadException: ..._STATICFIELD
Span<int>[] arr;                     // arrays não contêm ByRefLike (newarr -> TypeLoadException)
object o = span;                     // box -> InvalidProgramException (se T ByRefLike)
```

(Os dois primeiros erros de carga são os reais lançados em `methodtablebuilder.cpp`; os demais
seguem `byreflike-generics.md`. Em C# a maioria é barrada antes, em tempo de compilação.)

### Checklist de decisões

- [ ] Existe um tipo "só-stack"? Que regras de escape o compilador aplica?
- [ ] O runtime reverifica (carregador) ou confia no compilador?
- [ ] Como genéricos aceitam esses tipos (opt-in)?
- [ ] `readonly` é verificado ou só informativo?
- [ ] Buffers inline têm layout garantido?

## Referências

- `src/coreclr/vm/methodtable.h` (`enum_flag_IsByRefLike`, `IsByRefLike`)
- `src/coreclr/vm/methodtablebuilder.cpp` (~1401-1406, 1716-1723, 1801, 4210-4350, 9122)
- `docs/design/features/byreflike-generics.md`
- `docs/design/features/InlineArrayAttribute.md`
- `src/libraries/System.Private.CoreLib/src/System/Span.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IsByRefLikeAttribute.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IsReadOnlyAttribute.cs`
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/IsUnmanagedAttribute.cs`
