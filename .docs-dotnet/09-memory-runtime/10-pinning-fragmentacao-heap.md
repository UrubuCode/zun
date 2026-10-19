# Pinning e fragmentação de heap

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.

## Mecanismo no .NET

### O que é "pinning" e por que existe

Um GC com compactação **move** objetos. Se código nativo (P/Invoke, I/O assíncrono) ou código
`unsafe` guarda um endereço cru dentro de um objeto gerenciado, o objeto precisa ficar
**parado** durante esse tempo. "Fixar" (pin) um objeto proíbe o GC de movê-lo.

O comentário canônico está em `src/coreclr/gc/gcinterface.h` (tipo de handle `HNDTYPE_PINNED`):

> "Pinned handles are strong handles which have the added property that they prevent an object
> from moving during a garbage collection cycle. This is useful when passing a pointer to object
> innards out of the runtime while GC may be enabled. **NOTE: PINNING AN OBJECT IS EXPENSIVE AS
> IT PREVENTS THE GC FROM ACHIEVING OPTIMAL PACKING OF OBJECTS DURING EPHEMERAL COLLECTIONS.
> THIS TYPE OF HANDLE SHOULD BE USED SPARINGLY!**"

### Formas de fixar

| Forma | Onde | Nota |
|---|---|---|
| **Handle `Pinned`** | `GCHandle.Alloc(obj, GCHandleType.Pinned)` (`System/Runtime/InteropServices/GCHandle.cs`); `PinnedGCHandle<T>` (`PinnedGCHandle.T.cs`) | Só aceita objetos "pinnable" (`Marshal.IsPinnable`); vira um handle `HNDTYPE_PINNED` na handle table do GC (arquivo 12) |
| **Variável local fixada** (`fixed` em C#) | JIT: `lvPinned` em `importer.cpp`; slots marcados `GC_SLOT_PINNED` em `gcencode.cpp` | Ver abaixo |
| **`Memory<T>.Pin()`** | `System/Memory.cs:382` | Usa `GCHandle` Pinned, ou nada se o array já é pre-pinned |
| **Pinned Object Heap (POH)** | `GC.AllocateArray<T>(n, pinned: true)`, flag `GC_ALLOC_PINNED_OBJECT_HEAP` (`gcinterface.h:1102`) | Objetos nascem imóveis, numa "geração" própria (`poh_generation = 4`) |
| **NonGC heap / objetos congelados** | `docs/design/features/NonGC-Heap.md` | Objetos imortais fora do GC, nunca se movem nem são varridos |

### Locais fixados no JIT

O JIT marca variáveis locais fixadas e as descreve ao GC por *GC info* como slots com
`GC_SLOT_PINNED` (`src/coreclr/jit/gcencode.cpp:4197,4779`). Além disso, há uma otimização
recente: `Compiler::fgUnpinNonMovableLocals` (`src/coreclr/jit/lclmorph.cpp:2436`), fase
`PHASE_UNPIN_LOCALS` ("Unpin non-movable locals"), que **remove o pin** de locais cujo valor é
provadamente imóvel: constante, `LCL_ADDR`, objeto congelado, `GT_LCLHEAP`, ou uma cópia de outro
local nessas condições. Só roda com otimizações ligadas (`OptimizationDisabled()` retorna cedo),
e exclui parâmetros, locais com definições invisíveis e locais com endereço exposto.

### Como o GC lida com objetos fixados (plan phase)

No `plan_phase` (`src/coreclr/gc/plan_phase.cpp`), um objeto fixado forma um **"pinned plug"**
numa fila (`pinned_plug_que`, `deque_pinned_plug`, `oldest_pin`). Ao planejar a compactação, o
GC compacta os sobreviventes **em torno** dos plugs fixados, deixando-os no lugar; o espaço entre
eles vira "gaps" (`pinned_len`, `Align(min_obj_size)`).

Consequências visíveis no código:

- **Gerações misturadas**: um objeto fixado numa geração jovem pode permanecer no endereço onde
  está, e o GC precisa decidir se **promove** a "região" onde ele está. A função
  `decide_on_gen1_pin_promotion` (`src/coreclr/gc/allocation.cpp:5397`) é literalmente:

  ```cpp
  return ((pin_frag_ratio > 0.15) && (pin_surv_ratio > 0.30));
  ```

  onde `pin_frag_ratio = gen1_pins_left / total_space_to_skip` (fragmentação causada pelos pins)
  e `pin_surv_ratio = gen1_pins_left / dd_survived_size(gen1)` (peso dos pins na sobrevivência).
  Se verdadeiro, o GC "pula" os plugs fixados da gen1 e os atribui à geração mais velha
  (`attribute_pin_higher_gen_alloc`). **[confirmado]**
- **Contabilidade**: `dd_pinned_survived_size`, `dd_added_pinned_size`,
  `dd_artificial_pinned_survived_size` em `gcpriv.h` (linhas ~5801-5812) e
  `generation_pinned_allocation_sweep_size` / `..._compact_size`. "Artificial pinned" são
  objetos que o GC decide não mover por razões próprias (**[inferido]** pelo nome; não li a
  definição).
- **Demoção**: `gcpriv.h` tem `demotion`, `RI_DEMOTED`, `is_region_demoted`,
  `decide_on_demotion_pin_surv`: o GC pode **rebaixar** objetos (mantê-los na geração atual,
  sem promover) por causa de pins. Esse é o motivo da frase do BOTR de que "gen0 não é
  necessariamente mais novo que gen1".
- **O reaproveitamento do espaço entre pins**: `set_allocator_next_pin` e as `pinned_len`
  fazem o alocador aproveitar os "buracos" entre plugs fixados.
- **LOH**: `loh_pinned_queue` / `loh_pinned_queue_length` (`FEATURE_LOH_COMPACTION`) mostram que
  o LOH tem fila própria para compactação com pins (`compact_loh()`, `relocate_compact.cpp`).

### Fragmentação: o que é e como o GC reage

- **Fragmentação** = espaço livre (free objects) intercalado com objetos vivos. Medida por
  geração (`dd_fragmentation`).
- **Sweep** cria fragmentação (arquivo 02): espaço morto vira free objects na freelist
  (`min_free_list = 2*min_obj_size`).
- **Pins** são uma das principais causas de fragmentação do espaço efêmero: impedem o
  "empacotamento ótimo" (palavras do comentário em `gcinterface.h`).
- **Reação**: `decide_on_compacting` (`plan_phase.cpp:7933`) compacta quando
  `fragmentation >= dd_fragmentation_limit` **e** `fragmentation_burden >= dd_fragmentation_burden_limit`,
  ou quando a memória está sob pressão (arquivo 02). Se o espaço efêmero acaba, o GC pode
  aumentar o número de coletas gen1 (BOTR, "Determining which generation to collect").
- **LOH**: não compacta por padrão (BOTR e `settings.loh_compaction`); a fragmentação do LOH é
  um problema clássico; a compactação é opt-in e acontece em coletas gen2.
- **POH**: foi criado para que objetos fixados por longo prazo (buffers de I/O) **não
  fragmentem** gen0/gen1/gen2: eles vão a um heap separado, que nunca precisa pular plugs
  (`poh_generation`; `sweep_uoh_objects(poh_generation)` é sempre varrido, nunca compactado,
  em gen2). **[inferido]** quanto à motivação; a estrutura é confirmada.

### Tamanho do problema e proteção

`InteropValidatePinnedObjects` (`g_pConfig`, visto em `prestub.cpp:MayUsePrecompiledILStub`) é
uma opção para validar pins em interop. Existem também contadores/eventos de GC para
quantificar pins (`GCConfig`/eventos em `gcevents.h`).

## Por quê

- **Interop e I/O**: sem pin, passar buffers ao SO exigiria copiá-los para memória nativa.
  O desenho aceita o custo do pin para ter zero-cópia.
- **Mas o pin custa**: força a compactar "ao redor", cria buracos (fragmentação) e quebra a
  invariante "endereços baixos = objetos mais velhos". O GC então precisa de heurísticas de
  promoção/demoção (como as constantes 0.15 e 0.30).
- **POH e NonGC heap** surgiram para que o **caso comum de pins de longa duração** (buffers,
  literais) não pague esse preço no heap principal. O doc `NonGC-Heap.md` destaca benefícios
  para o GC e para o codegen: o JIT pode embutir endereços de objetos congelados.
- **Elisão de pins pelo JIT** (`fgUnpinNonMovableLocals`) corta pins redundantes que o
  programador não pode evitar com sintaxe.

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Pin temporário por handle/`fixed` | Zero-cópia, simples | Fragmenta o espaço jovem; custo de `GCHandle`; difícil de depurar vazamento de pin |
| POH | Pins de longa duração sem fragmentar | Mais um heap para dimensionar; sem compactação; coletado só em gen2 |
| Compactar ao redor de pins | Mantém compactação possível | Código complexo (plugs, filas, demoção); decisões heurísticas |
| Demotion / não-promoção | Evita promover lixo | Quebra a invariante geração = idade |
| LOH sem compactar | Barato | Fragmentação do LOH |
| Heap congelado (NonGC) | Sem custo de GC; código embute endereços | Só para objetos imortais; não liberável |
| Elisão automática de pins | Menos pins | Exige prova de imobilidade (frágil) |

## Como aplicar na sua linguagem

1. **Se seu GC move objetos, você precisa de uma API de pin.** Torne-a explícita, escopada e
   barata (um tipo "handle" descartável). O `MemoryHandle` e o `fixed` do .NET são bons modelos.
2. **Ofereça um heap "imóvel" (POH)** desde o começo, com alocação direta
   (`pinned: true`). É a melhor mitigação de fragmentação por pins de longa duração. Pode ser
   uma região separada, nunca compactada.
3. **Ofereça objetos imortais/estáticos fora do GC** (strings literais, tabelas): ganha-se
   código mais simples e GC mais rápido.
4. **Documente o custo de pin** no próprio nome da API. O .NET tem o aviso em caixa-alta no
   cabeçalho do tipo de handle.
5. **Se você implementar compactação**, projete uma "fila de pins" no plan phase e uma política
   de promoção de regiões com pins (ex.: >15% de fragmentação e >30% de pins na sobrevivência
   é o que o .NET usa para gen1 - ajuste para seu caso).
6. **Meça fragmentação por geração** e use limites duplos (absoluto e relativo) para decidir
   compactar, como em `dd_fragmentation_limit` e `dd_fragmentation_burden_limit`.
7. **Faça o compilador eliminar pins desnecessários**: se a variável aponta para algo
   imóvel (constante, pilha, objeto imortal), não registre o pin.
8. **Evite que o pin seja a única forma de interop**: arrays na memória nativa, `Span<T>` sobre
   `stackalloc` ou buffers de um `MemoryManager` (arquivo 08) reduzem a necessidade de pin.
9. **Tenha eventos/telemetria** que contem objetos fixados por GC; é o primeiro diagnóstico
   quando "o GC não compacta".
10. Considere **não ter compactação** (GC não-moving, como mark-sweep puro): elimina pinning
    por construção, ao custo de alocação por freelist e fragmentação permanente. É a decisão
    estrutural mais importante deste tópico.

## Referências

- `src/coreclr/gc/gcinterface.h` - `HNDTYPE_PINNED` e aviso; `GC_ALLOC_PINNED_OBJECT_HEAP`.
- `src/coreclr/gc/plan_phase.cpp` - fila de pinned plugs (`deque_pinned_plug`, `oldest_pin`),
  `decide_on_compacting`.
- `src/coreclr/gc/allocation.cpp:5397` - `decide_on_gen1_pin_promotion`;
  `attribute_pin_higher_gen_alloc`.
- `src/coreclr/gc/gcpriv.h` - `dd_pinned_survived_size`, `demotion`, `RI_DEMOTED`.
- `src/coreclr/gc/relocate_compact.cpp` - `compact_loh`, pinned plugs no relocate.
- `src/coreclr/jit/lclmorph.cpp:2436` - `fgUnpinNonMovableLocals`.
- `src/coreclr/jit/gcencode.cpp` - `GC_SLOT_PINNED`.
- `src/libraries/System.Private.CoreLib/src/System/Runtime/InteropServices/GCHandle.cs`.
- `src/libraries/System.Private.CoreLib/src/System/Memory.cs` - `Pin()`.
- `src/coreclr/System.Private.CoreLib/src/System/GC.CoreCLR.cs` - `AllocateArray(pinned)`.
- `docs/design/features/NonGC-Heap.md`.
- `docs/design/coreclr/botr/garbage-collection.md`.
