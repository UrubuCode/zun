# Spans e memória sem cópia (Span, ReadOnlySpan, Memory, ReadOnlyMemory)

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.
> Fontes principais: `src/libraries/System.Private.CoreLib/src/System/Span.cs`,
> `ReadOnlySpan.cs`, `Memory.cs`, `ReadOnlyMemory.cs`, `Buffers/MemoryManager.cs`.

## Mecanismo no .NET

### Span<T>: uma "visão" (view) de memória contígua

O comentário em `Span.cs` define: "Span representa uma região contígua de memória arbitrária.
Diferentemente de arrays, ela pode apontar para memória **gerenciada, nativa ou alocada na
pilha**. É type-safe e memory-safe."

Declaração real (`Span.cs:28-33`):

```csharp
public readonly ref struct Span<T>
{
    /// <summary>A byref or a native ptr.</summary>
    internal readonly ref T _reference;
    private readonly int _length;
    ...
}
```

Ou seja, um `Span<T>` são **dois campos**: uma **referência gerenciada interior (byref)** para
o primeiro elemento e um **comprimento**. Não há ponteiro para o objeto-dono nem para o
cabeçalho do array: a byref pode apontar para o *meio* de um objeto (ponteiro interior). O
`Span<T>` é marcado com `[Intrinsic]` e `[NonVersionable]` (o JIT conhece o tipo), e
`ReadOnlySpan<T>` tem a mesma forma, mas só dá leitura.

### Por que `ref struct`

`ref struct` é uma restrição de linguagem (C#) que diz que o valor só pode viver **na pilha**:
não pode ser campo de classe, ser boxed, capturado por lambda/async, nem entrar em heap.
Isso é o que torna a byref interior **segura**: o GC não precisa rastrear byrefs em objetos
do heap, e a byref nunca sobrevive ao frame que a criou (a regra exata da linguagem não está
no repositório do runtime; **[inferido]** pelo uso de `ref struct` e `ref T` em campo).

No lado do JIT, uma byref é um tipo de GC "TYP_BYREF" / `TYPE_GC_BYREF`, que o GC trata como
ponteiro interior (`docs/design/coreclr/jit/object-stack-allocation.md`, "GC considerations"):
referências a objetos do heap são reportadas como `TYPE_GC_REF`, e quando "pode ser heap ou
pilha" usa-se `TYPE_GC_BYREF`. O GC precisa então achar o objeto-base a partir de um ponteiro
interior (daí as estruturas `brick`/`plug` do coletor, arquivo 02) - **[inferido]**.

### Operações e segurança

`Slice` (Span.cs:396-430) é um bom exemplo de como a checagem de limites é feita com **uma
única comparação**:

```csharp
// TARGET_64BIT
if ((ulong)(uint)start + (ulong)(uint)length > (ulong)(uint)_length)
    ThrowHelper.ThrowArgumentOutOfRangeException();
return new Span<T>(ref Unsafe.Add(ref _reference, (nint)(uint)start), length);
```

Comentário no código: com `start`/`length` de 32 bits, a soma é calculada em 64 bits sem perda;
o cast para `uint` antes de `ulong` garante extensão por zero, de modo que valores negativos
ou overflow são capturados pela comparação com `_length`. Em 32 bits, usa-se a forma com
duas comparações "devido ao overhead da aritmética de 64 bits". O método é
`[MethodImpl(AggressiveInlining)]`, para que o JIT elimine o resto (arquivo 07).

`new Span<T>(T[] array)` usa `MemoryMarshal.GetArrayDataReference(array)`; se `array` for
covariante e seu tipo não for exatamente `T[]`, lança `ArrayTypeMismatchException`
(documentação em `Span.cs`), preservando a segurança de tipos.

### Memory<T>: a versão "armazenável"

`Memory<T>` (`Memory.cs:21-28`) é uma `readonly struct` **normal** (não `ref`), e por isso pode
ficar no heap e atravessar `await`:

```csharp
public readonly struct Memory<T>
{
    // highest order bit of _index: (_index < 0) => _object is a pre-pinned array
    private readonly object? _object;
    private readonly int _index;
    private readonly int _length;
}
```

Em vez de uma byref, guarda **o objeto dono** (`T[]`, `string` ou um `MemoryManager<T>`),
um índice e um comprimento - três campos. O `Memory<T>.Span` (propriedade) **reconstrói** a
`Span<T>` quando necessário: se o objeto é string, usa `GetRawStringData()`; se tem
"component size" (é array), usa a referência do array; caso contrário, é um `MemoryManager<T>`
e delega `GetSpan()`. Comentários no código mostram também o suporte especial a
`ReadOnlyMemory<char>` sobre `string` (inclusive a variância de arrays em ROM).

O **bit alto de `_index`** sinaliza que o array já está **pre-pinned** (**[inferido]**: tipicamente
alocado no Pinned Object Heap via `GC.AllocateArray(pinned: true)`, ver arquivo 10; o comentário
em `Memory.cs` só diz "pre-pinned array"). Nesse caso `Pin()` não aloca um `GCHandle`.

### Pin e interoperabilidade

`Memory<T>.Pin()` (`Memory.cs:382`) retorna `MemoryHandle`: "o GC não moverá a memória até que o
`MemoryHandle` retornado seja descartado". Para `string`/`T[]` não pre-pinned, faz
`GCHandle.Alloc(obj, GCHandleType.Pinned)` (arquivo 12); para `MemoryManager<T>`, delega a
`Pin(int)`.

### MemoryManager<T> e IMemoryOwner<T>

`MemoryManager<T>` (`Buffers/MemoryManager.cs`) é uma classe abstrata
(`IMemoryOwner<T>`, `IPinnable`) para quem quer **expor memória própria** (nativa, de um pool,
mapeada) como `Memory<T>`: implementa `GetSpan()`, `Pin(int)`, `Unpin()`, `Dispose(bool)`.
`Dispose` chama `GC.SuppressFinalize(this)` (arquivo 11).

### Alocar sem zerar, e já "pinned"

`GC.AllocateUninitializedArray<T>(int length, bool pinned = false)` e
`GC.AllocateArray<T>(int length, bool pinned = false)`
(`src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs:849,892`) permitem alocar arrays
sem zerar (a flag `GC_ALLOC_ZEROING_OPTIONAL` existe em `gcinterface.h:1100`) e/ou direto no
**Pinned Object Heap** (`GC_ALLOC_PINNED_OBJECT_HEAP`).

### stackalloc + Span (memória na pilha)

O `Span<T>` "pode apontar para memória alocada na pilha" (comentário de `Span.cs`). Isso
permite buffers temporários sem GC; o JIT também converte `localloc` fixo de ≤ 32 bytes em
variável local (`DEFAULT_MAX_LOCALLOC_TO_LOCAL_SIZE = 32`, `compiler.h:12678`). **[confirmado]**
nos fontes citados.

### O que o JIT/GC fazem por baixo

- `Span<T>` é tratada como struct de dois campos e pode ser **promovida** a registradores
  (fases "Struct promotion"/"Physical promotion", arquivo 07).
- `Unsafe.Add(ref ...)` e `MemoryMarshal.GetArrayDataReference` viram aritmética simples.
- Escrever uma referência para *dentro* de um `Span<T>` onde `T` é tipo-referência exige
  write barrier (arquivo 03), possivelmente da forma *checked*, já que a byref pode apontar
  para pilha, heap ou nativo (`gcIsWriteBarrierCandidate`).

## Por quê

- **Evitar cópias e alocações**: fatiar (`Slice`) é O(1) e não aloca; parsers, I/O, strings
  e serialização ficam sem `Substring`/`Array.Copy`. A doc do `ToArray`: "Isto aloca no heap,
  então deve ser geralmente evitado".
- **Unificar três tipos de memória** (gerenciada, nativa, pilha) em uma única abstração
  type-safe, sem `unsafe` espalhado.
- **Pressão de GC menor**: menos objetos temporários.
- **Duas formas** porque os dois casos têm restrições opostas: `Span<T>` (byref, só pilha,
  mais rápido) e `Memory<T>` (objeto+índice, pode ir ao heap/async, precisa ser "ligado" a um
  `Span` quando for usar).

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| `ref struct` com byref interior | Zero alocação, segura sem GC rastrear heap | Não pode ser campo de classe, nem usar em `async`/lambdas/iteradores; restringe a API |
| `Memory<T>` = (objeto, índice, comprimento) | Pode ser armazenado e passar por `await` | Mais lento ao obter o span (verificações de tipo do objeto) |
| Ponteiro interior | Fatia sem copiar | GC precisa resolver ponteiros interiores; custo no mark/relocate |
| Check de limites a cada `Slice`/indexação | Memory safety | Overhead (reduzido por inlining + range-check elimination) |
| `Pin()` via `GCHandle` | Interop com código nativo | Fixa objeto, causa fragmentação (arquivo 10); aloca um handle |
| Cast especial `ROM<char>` sobre `string` e variância | Ergonomia/perf | Código com "brechas" de type-safety só acessíveis por unsafe |
| `MemoryManager<T>` abstrato | Extensível (pools, nativo) | Chamadas virtuais; contrato de ciclo de vida (`Dispose`, `Unpin`) |

## Como aplicar na sua linguagem

1. **Decida se vale ter um tipo "view" de primeira classe.** Em linguagens com GC e arrays,
   é a melhor maneira de fatiar sem copiar. Defina-o como `{ponteiro_interior, comprimento}`.
2. **Dê ao tipo uma regra de vida (ownership/lifetime) enforçada pelo compilador.** O .NET
   usa a restrição "só vive na pilha" (`ref struct`). Se sua linguagem tiver análise de
   lifetime/borrow, pode relaxar isso. Sem ela, copie o modelo "stack-only".
3. **Tenha um tipo irmão armazenável** (como `Memory<T>`): objeto-dono + offset + length, e
   uma operação barata para "abrir" em view. Isso resolve o caso assíncrono/estruturas de dados.
4. **Faça ponteiros interiores um cidadão de primeira classe do GC e da IR** (tipos
   `GC_BYREF` distintos de `GC_REF`), com suporte no mapa de ponteiros e no write barrier
   (arquivo 03).
5. **Torne as operações centrais intrínsecas e inlináveis** (`Slice`, indexação, `Length`): são
   pequenas e quentes; o JIT precisa vê-las e promover os dois campos a registradores.
6. **Escreva checagem de limites em uma comparação** usando o truque de comparação sem sinal
   (`(uint)start + (uint)length > (uint)len`).
7. **Ofereça alocação "pinned" e "não zerada"** como parâmetros de alocação (como
   `GC.AllocateUninitializedArray(pinned)`): essencial para I/O e interop sem criar muitos
   handles. Planeje um *Pinned Object Heap* (arquivo 10).
8. **Interop**: defina `Pin()` com um objeto descartável (`MemoryHandle`) cujo `Dispose`
   solta o pin; documente que fixar é caro.
9. **Strings**: se strings são imutáveis, permita `ReadOnlySpan<char>` sobre elas sem cópia;
   considere se vai permitir "mutar" por brecha (o .NET permite, mas com unsafe).
10. **Extensibilidade de dono**: ofereça uma interface para fornecer memória de pools ou
    nativa (`MemoryManager<T>`), com contrato claro de `Pin/Unpin/Dispose`.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Span.cs` (449 linhas): `_reference`, `_length`,
  `Slice`.
- `.../System/ReadOnlySpan.cs`, `Memory.cs` (`Span`, `Slice`, `Pin`), `ReadOnlyMemory.cs`.
- `.../System/Buffers/MemoryManager.cs` - `MemoryManager<T>`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/MemoryMarshal.cs`,
  `GCHandle.cs`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` - `AllocateUninitializedArray`,
  `AllocateArray`.
- `src/coreclr/gc/gcinterface.h` - `GC_ALLOC_FLAGS` (`ZEROING_OPTIONAL`, `PINNED_OBJECT_HEAP`).
- `docs/design/coreclr/jit/object-stack-allocation.md` - `TYPE_GC_BYREF` e stack allocation.
- `src/coreclr/jit/gcinfo.cpp` - `gcIsWriteBarrierCandidate`.
