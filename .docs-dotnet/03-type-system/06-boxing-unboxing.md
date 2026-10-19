# Boxing/unboxing — custo e semântica

## Mecanismo no .NET

### 1. O que é um objeto boxed

Boxing converte um value type em um objeto de heap. O objeto boxed tem o **`MethodTable` do próprio
struct** como ponteiro de tipo (offset 0) e, logo depois, os bytes dos campos. Provas no código
gerenciado do CoreLib (`src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs`):

```csharp
internal static object Box(MethodTable* typeMT, ref byte unboxedData)
{
    Debug.Assert(typeMT->IsValueType);
    _ = Unsafe.ReadUnaligned<byte>(ref unboxedData);        // força NullReference se ref nula
    object boxed = RuntimeTypeHandle.InternalAllocNoChecks(typeMT);
    if (typeMT->ContainsGCPointers)
        Buffer.BulkMoveWithWriteBarrier(ref boxed.GetRawData(), ref unboxedData,
                                        typeMT->GetNumInstanceFieldBytesIfContainsGCPointers());
    else
        SpanHelpers.Memmove(ref boxed.GetRawData(), ref unboxedData, typeMT->GetNumInstanceFieldBytes());
    return boxed;
}
```

Ou seja: **uma alocação no heap do GC + uma cópia**; se o struct tem referências, a cópia usa
write barrier.

Esse helper é escolhido por `RuntimeHelpers.Box(MethodTable*, ref byte)`
(`RuntimeHelpers.CoreCLR.cs` ~475): `methodTable->IsNullable ? Box_Nullable : Box`.

### 2. Unboxing

```csharp
private static ref byte Unbox(MethodTable* toTypeHnd, object obj)
{
    // lança NullReferenceException se obj é null
    if (RuntimeHelpers.GetMethodTable(obj) == toTypeHnd)
        return ref obj.GetRawData();
    return ref Unbox_Helper(toTypeHnd, obj);
}
```

(`CastHelpers.cs` ~400). Pontos:
- `unbox` (a instrução IL) devolve uma **referência gerenciada (byref) aos dados dentro do box**,
  *sem copiar*; a cópia para o local é outra instrução (`ldobj`/`unbox.any`).
- O caminho rápido compara o `MethodTable` por identidade.
- O caminho lento `Unbox_Helper` (`CastHelpers.cs` ~666) aceita também *primitivos com o mesmo
  `CorElementType`* (ex.: unbox de um `int` boxed como enum de `int`, e vice-versa) e, com
  `FEATURE_TYPEEQUIVALENCE`, tipos equivalentes; senão `ThrowInvalidCastException`.
- Unbox de tipo errado → `InvalidCastException`; de `null` → `NullReferenceException`.

### 3. `Nullable<T>`: regra especial

`Nullable<T>` nunca vira um objeto "boxed Nullable<T>":
- Box: se `hasValue == false` → resultado é `null`; senão boxa o **`T` subjacente** (alocando o
  MethodTable de `T`). Código em `src/coreclr/vm/object.cpp` (~1357, `Nullable::Box`):
  `if (!*src->HasValueAddr(nullableMT)) return NULL; ... obj = AllocateObject(argMT); CopyValueClass(...)`.
  O comentário observa: "boxing should not trigger cctors" — aloca sem rodar o construtor estático.
- No JIT interface: `CEEInfo::getTypeForBox` devolve `T` quando o tipo é `Nullable<T>`
  (`src/coreclr/vm/jitinterface.cpp` ~6193) e `getBoxHelper` escolhe o helper adequado.
- Unbox para `Nullable<T>`: `CastHelpers.Unbox_Nullable` aceita `null` (→ `default`) ou um box de `T`
  (→ `Nullable<T>` com valor); caso contrário `InvalidCastException`
  (`CastHelpers.cs` ~612-660). Ver `09`.

### 4. Boxing implícito por interface / virtual

Value types não têm ponteiro de MethodTable; para virem a ser `object`, `ValueType`, `Enum` ou uma
interface é necessário boxing. Dois fatos importantes:
- A instrução IL **`constrained. callvirt`** evita boxing quando o tipo sobrescreve o método.
  `Ecma-335-Augments.md` (~linha 434) estende o texto: para *default interface methods* o boxing
  implícito **torna-se observável** (o método da interface recebe `this` como objeto/cópia).
- Mesmo assim, chamar um método herdado de `object`/`ValueType` sem override (ex.: `GetType()`)
  exige box.

### 5. Genéricos reduzem boxing

Com genéricos reificados (`02`), `List<int>` não boxa. Sem genéricos (ou em código compartilhado
sobre `__Canon`, que só vale para referências) boxing seria necessário. Em
`docs/design/features/byreflike-generics.md`, tipos ByRefLike nunca podem ser boxed: se `T` é
ByRefLike, `box` lança `InvalidProgramException`.

### 6. Custo (observável a partir do código)

- 1 alocação (`InternalAllocNoChecks`) + 1 cópia de `GetNumInstanceFieldBytes()` bytes.
- Pressão sobre o GC (objeto novo por box).
- Unbox é barato (compara MT e devolve ref) mas a *cópia* que normalmente segue é por valor.
- A identidade de objeto é nova a cada box: `object a = 1; object b = 1;` são dois objetos.
- Mutação de um struct dentro de um box afeta só o box (a cópia original é independente).
- `ValueType.Equals`/`GetHashCode` em struct "bit-comparável" é rápida; senão usa reflexão
  (`ValueType.cs`), o que combinado com boxing é um clássico custo escondido.

## Por quê

- **Unificação do sistema de tipos**: tudo é `object`. Para isso value types precisam de uma forma
  com cabeçalho de tipo, que só existe no heap.
- **Interop com código não genérico** (coleções antigas, `String.Format(object[])`, reflexão).
- **Custo pago só quando necessário**: structs puros não carregam cabeçalho.
- **`Nullable<T>` → `null`**: para que `(object)(int?)null == null`, mantendo a semântica dos
  tipos de referência; é a única exceção, por isso o runtime a trata na categoria própria
  (`enum_flag_Category_Nullable`).

## Trade-offs

| Ganho | Custo |
|---|---|
| Unificação de `object` | Alocação oculta no código |
| Structs sem header | `unbox` deve checar tipo exato (ou equivalente) |
| `Nullable<T>` com semântica de null | Código especial no runtime/JIT/helpers (4+ pontos) |
| `constrained.` evita box | Só no código especializado por valor (não no `__Canon`) |

Outros custos: aliasing surpreendente (boxes duplicados), `InvalidCastException` em unbox de
tipo "quase igual" (ex.: unbox `int` boxed como `long`), e armadilhas de mutação via interface.
O custo de GC é a razão pela qual APIs modernas do .NET usam genéricos, `Span<T>` e
`ref struct` (que nem podem ser boxed).

## Como aplicar na sua linguagem

1. **Se tiver `object` universal e value types**, defina boxing como "alocar + copiar" e documente
   isso como uma operação **explícita no IR** (`box`/`unbox`), mesmo que implícita no código-fonte.
   Isso permite análise de escape e eliminação de box (o JIT do .NET faz isso — conhecimento geral).
2. **Prefira não precisar de box**: genéricos reificados + interfaces via constraint (sem boxing) +
   chamadas "constrained".
3. **Unbox devolve uma referência, não uma cópia** — separa checagem de tipo de cópia de dados.
4. **Trate o tipo opcional especialmente no box** (`None` ↔ `null`) *só se* sua linguagem quiser
   a equivalência com null; do contrário, uma opção boxed comum é mais simples (e evita 4 casos
   especiais no backend).
5. **Proíba box de tipos que contêm referências a stack** (`ref struct`), fazendo-o erro de
   compilação e de verificação de IL.
6. **Defina a regra de unbox de tipos "quase iguais"**: o .NET permite enum<->tipo subjacente
   (mesmo `CorElementType`); decida isso conscientemente.
7. **Cuide da escrita**: se o struct tem referências, a cópia no box precisa de barreira de escrita.

### Exemplo: onde o boxing acontece

```csharp
int i = 5;
object o = i;            // box: alloc + cópia de 4 bytes
int j = (int)o;          // unbox.any: checa MT, copia 4 bytes
long k = (long)o;        // InvalidCastException (MT de int != long; Unbox_Helper só aceita mesmo CorElementType)
int? n = null;
object on = n;           // null (Nullable.Box: hasValue == false)
int? m = (int?)o;        // Unbox_Nullable: objeto é box de int -> hasValue = true
```

### Checklist de decisões

- [ ] Existe um supertipo universal (`object`)? Se sim, como value types chegam nele?
- [ ] `box`/`unbox` são operações explícitas no IR (permitem otimização)?
- [ ] Tipo opcional tem box especial?
- [ ] Quais tipos são proibidos de box (ref structs)?
- [ ] Chamadas via interface em structs evitam box em código especializado?

## Referências

- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/CastHelpers.cs` (`Box`, `Box_Nullable`, `Unbox`, `Unbox_Helper`, `Unbox_Nullable`)
- `src/coreclr/System.Private.CoreLib/src/System/Runtime/CompilerServices/RuntimeHelpers.CoreCLR.cs` (~475, ~640)
- `src/coreclr/vm/object.cpp` (`Nullable::Box`, `Nullable::UnBox`, ~1350-1460)
- `src/coreclr/vm/jitinterface.cpp` (`getTypeForBox`, `getBoxHelper`, ~6193)
- `src/coreclr/vm/methodtable.h` (`enum_flag_Category_Nullable`)
- `docs/design/specs/Ecma-335-Augments.md` (constrained., DIM)
- `docs/design/features/byreflike-generics.md`
- `src/coreclr/System.Private.CoreLib/src/System/ValueType.cs`
