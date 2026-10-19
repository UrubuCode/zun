# Criação na pilha: stackalloc e Span

## Mecanismo no .NET

### `stackalloc`

`stackalloc T[n]` aloca `n * sizeof(T)` bytes no **frame da pilha** do método atual. No IL é a
instrução `localloc`. Não passa pelo GC: a memória some quando o método retorna. Dois usos:

```csharp
unsafe { int* p = stackalloc int[16]; }          // ponteiro (contexto unsafe)
Span<int> s = stackalloc int[16];                 // seguro, sem unsafe (C# 7.2+)
Span<byte> buf = n <= 256 ? stackalloc byte[n] : new byte[n];   // padrão híbrido
```

No JIT o nó é `GT_LCLHEAP`; para tamanhos constantes pequenos o JIT pode convertê‑lo em um
local normal de bloco (`importer.cpp`, ~linhas 10366‑10385: "Converting stackalloc of N bytes
to new local", e "stackalloc of 0 bytes to push null unmanaged pointer"). Para tamanhos
variáveis, o prólogo ajusta o ponteiro de pilha e faz *stack probing* (tocar páginas para
disparar o guard page) para tamanhos grandes.

### Zeragem

Por padrão `localloc` é zerado se o método tem `localsinit` (`CORINFO_OPT_INIT_LOCALS`,
`corinfo.h` ~linha 713). `[SkipLocalsInit]` desliga e o conteúdo vira lixo → o programador
deve escrever antes de ler. (Atributo em
`src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/SkipLocalsInitAttribute.cs`.)

### `Span<T>` e `ref struct`

`Span<T>` (`src/libraries/System.Private.CoreLib/src/System/Span.cs`, linha 28) é um
`readonly ref struct` com dois campos: `ref T _reference` (ref field — um **managed
pointer** que pode apontar para o interior de array, pilha ou memória nativa) e `int
_length`. O ponteiro `ref` é um "interior pointer" que o GC sabe rastrear (descritores de
GC para byrefs: ver `gcdesc.h` e as flags `GC_CALL_INTERIOR` em `gcinterface.h` linha ~1087).

As regras de segurança de `ref struct` são impostas **pelo compilador** (análise de *ref
safety*/escopo):
- `ref struct` não pode ser campo de classe, nem boxed, nem capturado em lambda/async/iterator,
  nem usado como argumento de tipo genérico (até `allows ref struct` no C# 13);
- um `Span<T>` obtido de `stackalloc` não pode escapar do método (não pode ser
  retornado, atribuído a um span de escopo maior).

Isso garante em tempo de compilação que a referência para a pilha nunca sobrevive ao frame.
O runtime marca o tipo como *byref‑like* (flag `IsByRefLike` no `MethodTable`).

### Padrões de uso na BCL

- Limite de `stackalloc`: constante comum `StackallocThreshold` / `MaxStackallocThreshold =
  256` em `src/libraries/System.Private.CoreLib/src/System/Buffers/Text/Base64Decoder.cs`
  (linha 13, usada na ~107: `(uint)upperBound <= MaxStackallocThreshold ? stackalloc ... :
  ...`). Outros arquivos: `Base64UrlDecoder.cs`, `IdnMapping.Icu.cs`.
- Builders com buffer inicial na pilha e fallback para `ArrayPool` quando crescem
  (`ValueStringBuilder`/`ValueListBuilder` no código compartilhado das bibliotecas — o
  padrão: `new ValueStringBuilder(stackalloc char[256])`), combinando arquivos 10 e 11.
- `InlineArray` (`System.Runtime.CompilerServices.InlineArrayAttribute`) permite arrays de
  tamanho fixo embutidos em structs (ex.: buffers de 8 elementos) sem `unsafe`.
- `GC.AllocateUninitializedArray` e `Span<T>.Slice` são alternativas para heap.

### Objetos na pilha (escape analysis)

Além de `stackalloc`, o JIT do .NET recente faz *object stack allocation* quando prova que
um `new` não escapa — `src/coreclr/jit/objectalloc.cpp`. Isso é otimização transparente,
sem mudança de sintaxe, e restrita a casos simples (tipos sem finalizador, sem escape
via retorno/campo/chamada não analisada).

### Limites da pilha

A pilha de thread tem tamanho fixo (1 MB padrão em Windows; 8 MB típico em Linux — valores do
SO/config). `stackalloc` excessivo causa `StackOverflowException`, que **não é capturável**
e derruba o processo. Por isso a convenção de limites pequenos (256 B‑1 KB).

## Por quê

- **Custo ~zero**: alocar é decrementar SP; liberar é automático; sem pressão no GC; ótima
  localidade de cache.
- **Segurança por tipo**: `Span` encapsula (ponteiro, tamanho) com bounds‑checking, evitando
  buffer overflow do `T*` cru.
- **Uniformidade**: o mesmo `Span<T>` representa array, fatia de array, memória nativa e
  pilha — algoritmos escrevem‑se uma vez.
- **ref struct** transfere a prova de tempo de vida ao compilador, sem custo de runtime.

## Trade-offs

- Tamanho limitado e sem tratamento de falha (estouro = crash fatal).
- Não pode escapar do método: o padrão precisa de fallback para heap/pool quando o tamanho é
  desconhecido, com ramos extras.
- `ref struct` limita a linguagem: sem async/iterators/lambdas/campos de classe/genéricos
  tradicionais — muita complexidade nas regras de escopo (ref safety v2 no C# 11).
- `SkipLocalsInit` + `stackalloc` pode vazar dados antigos da pilha (segurança).
- `localloc` em loops acumula (só libera no retorno do método).
- Prejudica inlining em alguns casos (o JIT não inlina métodos com `localloc`).

## Como aplicar na sua linguagem

1. Ofereça uma primitiva de **alocação de escopo de função** (`localloc`/`alloca`) com tipo
   seguro de "view" (`Span`/slice) e proíba a view de escapar do escopo, via análise de
   tempo de vida no compilador (como `ref struct`) ou via sistema de ownership (Rust
   lifetimes).
2. Exponha uma única abstração de **fatia (ptr+len)** que cubra heap gerenciado, pilha e
   nativo — é a maior alavanca de design do `Span<T>`.
3. Defina **limites e fallback**: documente um limiar (padrão da BCL: 256 elementos) e
   ofereça utilitários que escolham pilha vs. pool.
4. Trate o estouro de pilha como erro fatal claro; considere verificar tamanho em
   runtime (`RuntimeHelpers.TryEnsureSufficientExecutionStack` existe no .NET).
5. Implemente **escape analysis** no otimizador para promover alocações pequenas e não
   escapantes à pilha automaticamente, evitando que o programador use APIs especiais.
6. Se tiver `ref struct`‑like, decida cedo a interação com async, closures e genéricos; é
   a parte mais cara de evoluir depois.
7. Zere por padrão e permita `skip‑init` explícito e inseguro.

## Detalhes adicionais

### Exemplo completo (padrão da BCL)

```csharp
const int StackLimit = 256;
byte[]? rented = null;
Span<byte> buffer = length <= StackLimit
    ? stackalloc byte[StackLimit]
    : (rented = ArrayPool<byte>.Shared.Rent(length));
try { Process(buffer.Slice(0, length)); }
finally { if (rented != null) ArrayPool<byte>.Shared.Return(rented); }
```

Mostra a união dos arquivos 10 e 11: pilha para o caso comum, pool para o raro.

### Detalhes de `localloc` no JIT

- O `localloc` obriga o método a ter frame com ponteiro de frame porque o SP varia.
- Em tamanho constante e pequeno, o importador troca por local (`importer.cpp`
  ~10382-10385) e possibilita promoção/otimização.
- A pilha precisa de probes em alocações grandes para tocar cada página (implementado no
  codegen do JIT; arquivos exatos não verificados).

### Span sobre outras fontes

`Span<T>` pode apontar para: array (`MemoryMarshal.GetArrayDataReference`, `Span.cs` linhas
~52/89), `stackalloc`, memória nativa (`new Span<T>(void*, int)`), campo de struct ou
`InlineArray`. `Memory<T>` é a contraparte heap-safe (não é `ref struct`).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Span.cs` — `Span<T>`.
- `src/libraries/System.Private.CoreLib/src/System/Buffers/Text/Base64Decoder.cs` — padrão
  `MaxStackallocThreshold`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/CompilerServices/InlineArrayAttribute.cs`,
  `SkipLocalsInitAttribute.cs`.
- `src/coreclr/jit/importer.cpp` (~10366) — tratamento de `localloc`; `src/coreclr/jit/objectalloc.cpp`.
- `src/coreclr/inc/corinfo.h` — `CORINFO_OPT_INIT_LOCALS`.
- Documentação de design de ref safety está no repositório `dotnet/csharplang` (externo).
