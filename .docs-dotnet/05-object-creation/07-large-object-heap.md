# Large Object Heap (LOH) — objetos grandes

## Mecanismo no .NET

### Limiar

`src/coreclr/gc/gc.h` (linha 106) e `gcinterface.h` (linha 246) definem
`#define LARGE_OBJECT_SIZE ((size_t)(85000))`. Objetos de **85.000 bytes ou mais** vão
para o LOH. Em `gc.cpp` (linha ~35) existe `size_t loh_size_threshold = LARGE_OBJECT_SIZE;`,
e `src/coreclr/gc/gcconfig.h` (linha ~82) expõe a configuração
`GCLOHThreshold` / `System.GC.LOHThreshold` ("Specifies the size that will make objects go
on LOH"), com padrão `LARGE_OBJECT_SIZE`. Ou seja, o limiar é ajustável (para cima).

O `docs/design/coreclr/botr/garbage-collection.md` (linhas ~27-37) confirma: "small objects
(< 85,000 bytes) and large objects (>= 85,000 bytes)"; large objects não usam allocation
contexts nem quanta — "são alocados diretamente em um segmento do heap".

### Como é alocado

- Para arrays, `AllocateSzArray` em `src/coreclr/vm/gchelpers.cpp` calcula `totalSize` e
  passa para `Alloc(totalSize, flags)`; o GC decide pelo tamanho (>= limiar) usar o heap
  de objetos grandes (UOH). Chamadores também podem pedir explicitamente
  `GC_ALLOC_LARGE_OBJECT_HEAP` (flag em `gcinterface.h` ~linha 1101).
- Dentro do GC o heap "user old" tem duas variantes: **LOH** e **POH** (Pinned Object
  Heap, `GC_ALLOC_PINNED_OBJECT_HEAP`), agrupados em `GC_ALLOC_USER_OLD_HEAP`. O POH é
  usado por `GC.AllocateArray<T>(length, pinned: true)` (`GC.CoreCLR.cs` ~linha 892).
- O limite de tamanho de array vem de `MaxArrayLength()` em `gchelpers`
  (comentário "Keep in sync with `inline SIZE_T MaxArrayLength()` from gchelpers" em
  `src/libraries/System.Private.CoreLib/src/System/Array.cs` ~linha 2671).

### Geração e coleta

O doc BOTR (linhas ~85-110): para objetos pequenos existem gen0, gen1, gen2; para grandes,
"há uma geração — gen3" (internamente `loh_generation`, depois a POH). Alocação de LOH
"sempre vai para gen3". O LOH é coletado **junto com gen2** (uma GC gen2/full inclui LOH);
uma coleta de gen0/gen1 não varre o LOH. Alocar muito no LOH pode, portanto, disparar GCs
gen2 (o LOH tem seu próprio orçamento que, ao estourar, dispara gen2).

### Compactação

Por padrão o LOH **não é compactado**: livre‑se por *sweep* e mantém‑se uma free‑list.
Compactar é opcional e sob demanda: `GCSettings.LargeObjectHeapCompactionMode =
GCLargeObjectHeapCompactionMode.CompactOnce` (afeta a próxima gen2 blocking) — API em
`System.Runtime.GCSettings`. A configuração `GCLOHCompact` também existe. (Em versões com
*regions*, o LOH usa regiões grandes próprias.) Memória livre do LOH é reutilizada por
alocações de tamanho compatível; pode haver fragmentação.

### Zeragem e custos

Objetos de LOH são zerados na alocação (a menos que `ZEROING_OPTIONAL`, como em
`AllocateUninitializedArray` para arrays grandes de tipos sem refs). Alocar e zerar 100 KB+
é caro; por isso se recomenda **reuso** (`ArrayPool`, arquivo 11).

### Frozen Object Heap

`gchelpers.cpp` (~linha 644) mostra `foh->TryAllocateObject(...)`: existe o *Frozen Object
Heap* para objetos imutáveis alocados fora do GC (strings literais, `typeof`, arrays
estáticos `static readonly` iniciais) que nunca são movidos nem coletados. É um caminho
alternativo de alocação para casos especiais, não relacionado ao LOH por tamanho.

## Por quê

- **Copiar é caro**: o GC compactador move sobreviventes; mover blocos de MBs a cada gen0/1
  seria proibitivo. Separar objetos grandes evita o custo, e por isso são tratados como
  "velhos" desde o início (tratados como gen2‑like).
- **Evitar poluir gen0**: um objeto de 1 MB consumiria o orçamento de gen0 e causaria GCs
  frequentes; também não caberia num allocation quantum de ~8 KB.
- **Sweep em vez de compactar** reduz pausas, ao custo de fragmentação.
- **Limiar configurável**: permite ajustar a cargas com muitos arrays intermediários.

## Trade-offs

- Objetos de curta duração no LOH são caros: só são recuperados em gen2 (e o gen2 é a
  coleta mais cara). O clássico padrão ruim: `new byte[100_000]` por requisição.
- Fragmentação do LOH → `OutOfMemoryException` mesmo com memória livre total suficiente
  (falta bloco contíguo).
- Zerar blocos grandes consome tempo e banda.
- Limiar fixo (85 KB) é um número mágico; arrays de `double[10_600]` caem no LOH e
  `double[10_000]` não — surpreende.
- Mais um heap significa mais complexidade no GC (cartões, bg GC, compactação opcional).

## Como aplicar na sua linguagem

1. Se o GC for copiante/de gerações, defina um **espaço de objetos grandes** com política
   própria (não copiante, coleta junto com a geração velha). O limiar deve ser uma constante
   configurável; escolha com base no tamanho do bloco/quantum de alocação (aqui ~85 KB vs.
   quantum ~8 KB — referência de ordem de grandeza).
2. Faça objetos grandes alocarem **fora do TLAB**, direto do heap, e registre
   `alloc_bytes_uoh` separado para observabilidade.
3. Ofereça APIs de **reuso** (pools) e de **alocação sem zeragem** para tipos sem
   referências, como `AllocateUninitializedArray`.
4. Dê controle de compactação sob demanda (opt‑in), pois a compactação do espaço grande é
   cara mas resolve fragmentação.
5. Ofereça um espaço **pinned** (POH) para buffers de I/O/interop, evitando que objetos
   fixados quebrem a compactação do heap normal.
6. Considere um heap *frozen/imutável* para constantes de programa (strings literais) para
   tirá‑las do GC (o NativeAOT e o CoreCLR fazem).
7. Documente claramente a regra do limiar; programadores otimizam em torno dela.

## Detalhes adicionais

### Exemplo prático

```csharp
var a = new byte[84_900];   // total < 85.000 bytes: SOH, gen0
var b = new byte[85_000];   // total >= 85.000 bytes: LOH
```

O que conta é o **tamanho total do objeto**, inclusive cabeçalho de array, não só o
payload; por isso o corte efetivo para `byte[]` fica ligeiramente abaixo de 85.000
elementos (dedução a partir de `LARGE_OBJECT_SIZE` expresso em bytes de objeto; o cálculo
exato está em `gc.cpp`).

### Sintomas e diagnóstico

- Muitas GCs gen2 com pouca memória gen2 útil: sinal de churn de LOH.
- `GC.GetGCMemoryInfo()` fornece tamanhos e fragmentação por geração, incluindo LOH.

### Estratégias de mitigação

1. Reusar buffers (`ArrayPool<T>.Shared`, arquivo 11).
2. Alocar em pedaços menores que o limiar (listas de blocos, `ArrayBufferWriter`,
   `ReadOnlySequence<T>`).
3. Aumentar `GCLOHThreshold` para mover objetos médios ao SOH (trade-off: mais cópia).
4. Pedir compactação ocasional do LOH (`GCSettings.LargeObjectHeapCompactionMode`,
   `src/libraries/System.Private.CoreLib/src/System/Runtime/GCSettings.cs` linha 53;
   config `GCLOHCompact` em gcconfig.h linha 81).
5. Usar `GC.AllocateUninitializedArray` para evitar zerar.

### Checklist de projeto

- Constante do limiar e sua relação com o tamanho do quantum de alocação.
- Política de coleta do espaço grande (junto da geração mais velha).
- Compactação: nunca, sob demanda ou automática.
- Zeragem: obrigatória ou opcional por tipo sem referências.
- Espaço pinned separado para buffers de interop.
- Telemetria separada de bytes alocados fora do espaço pequeno (`alloc_bytes_uoh`).
- Limite máximo de tamanho de array (`MaxArrayLength`) e erro de OOM bem definido.

- Interação com Server GC e GC em background: o LOH participa de coletas gen2 em background (conceito; detalhes em gc.cpp/background.cpp).

## Referências

- `src/coreclr/gc/gc.h` (linha 106), `src/coreclr/gc/gcinterface.h` (linhas 246, ~1090-1103).
- `src/coreclr/gc/gcconfig.h` (linha 82) — `GCLOHThreshold`.
- `src/coreclr/gc/gc.cpp` (linha 35, `loh_size_threshold`), `src/coreclr/gc/allocation.cpp`.
- `src/coreclr/vm/gchelpers.cpp` — `AllocateSzArray`, frozen object heap (`TryAllocateObject`).
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` — `AllocateArray`,
  `AllocateUninitializedArray`.
- `docs/design/coreclr/botr/garbage-collection.md` — seções "Allocation", "Generations",
  "Large objects".
