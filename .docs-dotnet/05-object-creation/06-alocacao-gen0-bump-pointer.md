# Alocação no gen0: bump pointer e allocation context

## Mecanismo no .NET

### Visão geral em camadas

```
 new Foo()
   -> helper JIT (CORINFO_HELP_NEWSFAST etc.)   fast path em assembly/C, sem lock
        -> bump no alloc_context da thread       alloc_ptr += size   se <= alloc_limit
        -> (esgotou) slow path: AllocateObject / Alloc em gchelpers.cpp
             -> IGCHeap::Alloc(acontext, size, flags)   em src/coreclr/gc
                  -> pega novo "allocation quantum" do gen0, zera a memória, retorna
                  -> (sem espaço) dispara GC gen0/gen1/gen2
```

### gc_alloc_context

Definido em `src/coreclr/gc/gcinterface.h` (~linhas 198-225):

```cpp
struct gc_alloc_context {
    uint8_t* alloc_ptr;      // próximo byte livre
    uint8_t* alloc_limit;    // fim da região zerada/reservada
    int64_t  alloc_bytes;    // bytes alocados em SOH por este contexto
    int64_t  alloc_bytes_uoh;// bytes alocados fora do SOH (LOH/POH)
    void*    gc_reserved_1, gc_reserved_2;  // uso interno do GC
    int      alloc_count;
};
```

O comentário no mesmo arquivo: "Every Thread has its own allocation context that it hands to
the GC when allocating". Cada thread gerenciada possui um contexto; o VM o guarda em
`ee_alloc_context` (uso em `Alloc(ee_alloc_context*, size, flags)` em
`src/coreclr/vm/gchelpers.cpp` ~linha 270).

### Fast path: bump pointer

Os helpers de alocação (CoreCLR: assembly por plataforma/`JIT_NewS_MP_FastPortable`;
NativeAOT: `RhpNewFast` em `src/coreclr/nativeaot/Runtime/portable.cpp` linha ~57) fazem
essencialmente:

```
obj = ctx.alloc_ptr
novo = obj + tamanho
se novo > ctx.alloc_limit  ->  slow path
ctx.alloc_ptr = novo
obj->MethodTable = mt
retorna obj
```

Sem lock, sem CAS: o contexto é exclusivo da thread. A memória entre `alloc_ptr` e
`alloc_limit` já foi **zerada** pelo GC, então o objeto retorna com campos zero e com o
ObjHeader do próximo objeto já zerado (comentário em `syncblk.h`, arquivo 02).

### Slow path: allocation quantum

O `docs/design/coreclr/botr/garbage-collection.md` (linhas ~21-44) descreve:

- *Allocation contexts* são regiões dedicadas a uma thread dentro do heap; em máquina de 1
  processador lógico existe um único contexto.
- *Allocation quantum*: quantidade de memória que o alocador entrega por vez; "tipicamente
  8k" (e o tamanho médio de objeto gerenciado em torno de 35 bytes), de modo que um quantum
  atende muitos objetos.
- Benefícios documentados: **sem lock** enquanto o contexto não esgota; **localidade de
  cache** (objetos alocados em sequência ficam contíguos); zeragem em bloco.
- Objetos grandes não usam contexts/quanta (ver arquivo 07).

Quando o quantum esgota, `gc_heap::allocate_more_space` e afins (em `src/coreclr/gc/gc.cpp`
e `allocation.cpp`) pegam um novo bloco do gen0 (a free‑list ou espaço contíguo no final do
gen0), zeram o bloco (a menos que `GC_ALLOC_ZEROING_OPTIONAL`) e atualizam `alloc_ptr` /
`alloc_limit`. Se não há espaço no orçamento do gen0 (`gen0 budget`), o GC é disparado.

### Flags de alocação

`GC_ALLOC_FLAGS` (gcinterface.h ~linha 1090): `GC_ALLOC_FINALIZE`, `GC_ALLOC_CONTAINS_REF`,
`GC_ALLOC_ALIGN8_BIAS`, `GC_ALLOC_ALIGN8`, `GC_ALLOC_ZEROING_OPTIONAL`,
`GC_ALLOC_LARGE_OBJECT_HEAP`, `GC_ALLOC_PINNED_OBJECT_HEAP`. O enum é espelhado em
`src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` ("keep in sync with
GC_ALLOC_FLAGS").

### Coleta e o bump pointer

Gen0 é pequeno (orçamento dependente do tamanho de cache L3, ajustado dinamicamente). Em um
GC gen0, sobreviventes são promovidos para gen1 (compactação ou *sweep*), e o espaço de gen0
volta a ser uma região contígua zerada; os alloc contexts são "fixados" com
`FixAllocContext` (gcinterface.h linha ~879) para o GC reaproveitar o espaço restante dos
contexts parciais. Ver `docs/design/coreclr/botr/garbage-collection.md`, que descreve gens,
`regions`/`segments` e o ciclo mark/plan/relocate/compact.

### Zeragem opcional

`GC.AllocateUninitializedArray<T>` (`src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs`
~linha 849) pede `GC_ALLOC_ZEROING_OPTIONAL`: para tipos sem referências e arrays com pelo
menos `2048 / sizeof(T)` elementos (em build não‑Debug), o GC pode pular a zeragem. Arrays
menores usam `new T[]` normal "pois geralmente é mais rápido".

## Por quê

- **Alocação ~ custo de somar um ponteiro**: com um GC compactador (copia sobreviventes),
  todo o espaço livre é contíguo, então alocar é só avançar um ponteiro — muito mais barato
  que `malloc` com free‑lists.
- **Contexto por thread** elimina contenção: a sincronização só ocorre a cada ~8 KB.
- **Zerar em bloco** é mais rápido (memset/instruções vetoriais) e sai do caminho quente.
- **Gen0 pequeno** cabe no cache: os objetos recém‑criados estão quentes; a hipótese
  geracional (maioria morre jovem) torna a coleta de gen0 barata (custo proporcional aos
  vivos, não aos mortos).

## Trade-offs

- Exige GC *moving/compacting*; incompatível com ponteiros crus estáveis (daí `pinned`,
  `fixed`, `GCHandle.Alloc(Pinned)`, POH).
- Cada thread retém até um quantum não usado (fragmentação interna/pico de memória com
  muitas threads).
- Zeragem obrigatória custa largura de banda; o GC oferece opt‑out apenas em casos
  restritos.
- Barreiras de escrita (card marking) são o preço do GC geracional: toda escrita de
  referência em objeto antigo paga `JIT_WriteBarrier`.
- Pausas "stop‑the‑world" em gen0 (curtas) vs. throughput.

## Como aplicar na sua linguagem

1. Se a linguagem tem GC, **projete para bump allocation**: GC copiante/compactante de
   gerações, ao menos "nursery" copiante + old gen mark‑sweep. É a mudança de maior ganho na
   taxa de alocação.
2. Use **TLAB** (thread‑local allocation buffer) = `alloc_ptr/alloc_limit` por thread;
   fast path inline de 4‑5 instruções no código gerado ou helper curto.
3. Defina o tamanho do quantum (8 KB é o valor citado na doc do .NET; HotSpot usa TLABs
   adaptativas) e zere o bloco inteiro ao entregá‑lo.
4. Reserve flags de alocação (finalizável, contém refs, alinhamento, zero opcional, heap
   grande/pinned) numa interface estreita entre VM e GC (como `IGCHeap::Alloc`) para poder
   trocar o GC.
5. Registre `alloc_bytes` por contexto para telemetria barata (`GC.GetAllocatedBytesForCurrentThread`).
6. Sem GC (Rust/C), ainda é útil um **arena/bump allocator** com `reset` em lote para
   escopos curtos (AST de compilador, request handler).
7. Considere escape analysis para eliminar alocações antes mesmo de chegar ao bump pointer.

## Detalhes adicionais

### Fluxo detalhado do slow path

1. O helper rápido falha (`alloc_ptr + size > alloc_limit`) e chama o slow path em
   `gchelpers.cpp` (`Alloc(ee_alloc_context*, size, flags)` linha ~270), que chama
   `GetGCHeap()->Alloc(pAllocContext, size, flags)` (linha ~327).
2. No GC (`gc.cpp`/`allocation.cpp`), o alocador tenta encaixar a requisição na free list ou
   no espaço contíguo restante do gen0, zera a região entregue e devolve o intervalo ao
   contexto (nomes internos exatos de função não verificados aqui).
3. Se o orçamento de alocação do gen0 acabou, uma coleta é disparada.
4. Em modo servidor (Server GC) há um heap por core lógico e cada thread usa o heap de
   afinidade (`IsThreadUsingAllocationContextHeap`, `gcinterface.h` linha ~869), reduzindo
   contenção.

### Números confirmados na doc

- Quantum típico: 8 KB; tamanho médio de objeto gerenciado: ~35 bytes (BOTR GC).
- `gc_alloc_context.alloc_count` e `alloc_bytes` permitem estatística por thread.

### Alocação alinhada

`GC_ALLOC_ALIGN8` e `GC_ALLOC_ALIGN8_BIAS` existem para tipos que precisam de alinhamento
em 8 bytes (ex.: `double[]` em 32 bits). Em 64 bits o alinhamento natural já é 8
(`AllocAlign8` aparece em `gcpriv.h`).

### Observabilidade

`GC.GetAllocatedBytesForCurrentThread()` e eventos de amostragem de alocação expõem a taxa
sem custo no fast path graças aos contadores `alloc_bytes` do contexto.

## Referências

- `src/coreclr/gc/gcinterface.h` — `gc_alloc_context`, `GC_ALLOC_FLAGS`, `IGCHeap::Alloc`,
  `FixAllocContext`.
- `src/coreclr/gc/gc.cpp`, `src/coreclr/gc/allocation.cpp`, `src/coreclr/gc/gcpriv.h`.
- `src/coreclr/vm/gchelpers.cpp` — `Alloc`, `AllocateObject`, `AllocateSzArray`.
- `src/coreclr/nativeaot/Runtime/portable.cpp` — `RhpNewFast`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` — `AllocateUninitializedArray`.
- `docs/design/coreclr/botr/garbage-collection.md` — allocation context, quantum, gerações.
