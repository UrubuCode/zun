# Server GC vs Workstation GC

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.

## Mecanismo no .NET

O CoreCLR tem **dois sabores de GC**, selecionados na inicialização do processo:

- **Workstation (WKS)**: um heap, a coleta roda na própria thread de usuário que estourou o
  orçamento.
- **Server (SVR)**: **um heap por (conjunto de) processador(es)** e **threads de GC dedicadas**
  que trabalham em paralelo em cada heap.

Escolha por configuração em `src/coreclr/gc/gcconfig.h`:

| Config | Nome | Padrão |
|---|---|---|
| `ServerGC` | `gcServer` / `System.GC.Server` | `false` |
| `ConcurrentGC` | `gcConcurrent` / `System.GC.Concurrent` | `true` |
| `HeapCount` | `GCHeapCount` / `System.GC.HeapCount` | 0 (automático) |
| `GCHeapAffinitizeMask` | máscara de CPUs para as threads de GC | 0 |
| `GCHeapAffinitizeRanges` | lista/intervalos de CPUs | - |
| `NoAffinitize` | `GCNoAffinitize` - não fixar threads de GC a CPUs | `false` |
| `GCCpuGroup` | usar CPU groups | `false` |
| `GCDynamicAdaptationMode` | `System.GC.DynamicAdaptationMode` | **1** |
| `GCConserveMem` | "quão forte o GC tenta conservar memória" (0-9) | 0 |
| `GCHeapHardLimit` / `GCHeapHardLimitPercent` | teto do heap | 0 |

### Um só código-fonte, duas compilações

O mesmo conjunto de `.cpp` em `src/coreclr/gc/` é compilado duas vezes, em namespaces
diferentes. Aparece no começo de `finalization.cpp`, `dynamic_heap_count.cpp` etc.:

```cpp
#ifdef SERVER_GC
namespace SVR {
#else // SERVER_GC
namespace WKS {
#endif
```

`MULTIPLE_HEAPS` está ativo na compilação SVR; muitas rotinas trazem variantes
`#ifdef MULTIPLE_HEAPS` (por exemplo, `sc.thread_number = hp->heap_number` e
`sc.thread_count = gc_heap::n_heaps`, vistos em `CFinalize::ScanForFinalization`).
**[confirmado]**

### Fluxo comparado (BOTR, "Code Flow")

**WKS, não concorrente**: a thread de usuário estoura o orçamento -> `SuspendEE` -> decide a
geração -> mark -> plan -> relocate+compact ou sweep -> `RestartEE`.

**WKS concorrente (padrão)**: a thread de usuário suspende o EE, decide fazer **background
GC**, acorda a thread de background (`.NET BGC`, criada em `background.cpp:2853`), que
chama `RestartEE`. As threads gerenciadas seguem alocando enquanto o background GC marca e
varre. Se estourar o orçamento de gen0/gen1, ocorre um GC "foreground" no meio.

**SVR, não concorrente**: a thread de usuário estoura o orçamento -> as **threads de GC do
Server são acordadas**, chamam `SuspendEE`, fazem o trabalho (mesmas fases) e
chamam `RestartEE`.

**SVR concorrente**: igual ao WKS concorrente, exceto que os GCs não-background rodam nas
threads de GC do Server.

### Diferenças de tamanho e política

- **gen0 budget**: `get_gen0_min_size()` (`init.cpp:1158`) usa
  `max(4*cache/5, 256KB)` no WKS e `max(cache, 256KB)` no SVR, onde `cache` =
  `GetCacheSizePerLogicalCpu`. Em SVR o produto `gen0size * n_heaps` não pode passar de
  1/6 da memória física (reduz pela metade até caber).
- **Work stealing no mark**: `gc_heap::mark_steal()` (`mark_phase.cpp:~1970`), balanceando
  carga entre threads de heap quando um heap tem mais objetos a marcar.
- **Uma fila de finalização por heap** (`finalize_queue` é `PER_HEAP_FIELD`, `gcpriv.h:3916`);
  há funções para mesclar/dividir quando o número de heaps muda
  (`CFinalize::MergeFinalizationData`, `SplitFinalizationData`).
- **Allocation contexts** por thread de usuário existem nos dois; no Server cada thread de
  usuário tem afinidade com um heap (**[inferido]** - o BOTR diz apenas que em máquina de 1
  processador há um único contexto de gen0).

### DATAS: número de heaps dinâmico

`src/coreclr/gc/dynamic_heap_count.cpp` (1549 linhas), sob `USE_REGIONS` e
`DYNAMIC_HEAP_COUNT`, implementa **mudança dinâmica do número de heaps** do Server GC em
tempo de execução. O código contém o conceito de heap "decomissionado"
(`check_decommissioned_heap`), e `GCConfig` expõe `GCDTargetTCP` ("target tcp for DATAS").
Em `init.cpp:~1190`, no modo `dynamic_adaptation_to_application_sizes`, o tamanho da gen0 é
limitado a **4 MB** ("se estamos sendo avarentos com memória"). `GCDynamicAdaptationMode`
tem padrão 1, que corresponde a `dynamic_adaptation_to_application_sizes` (enum em
`gcpriv.h:600`). **[confirmado]** nos símbolos; o sigla "DATAS" aparece no texto de
`gcconfig.h`. O comportamento detalhado (quando e quanto ele cresce/encolhe) **não foi
investigado aqui**.

### Limite rígido de memória

`GCHeapHardLimit` ajusta o número de heaps: `adjust_heaps_hard_limit(nhp)` em `init.cpp`
reduz `nhp` para que cada heap tenha tamanho mínimo; com hard limit a gen0 fica limitada a
um oitavo do segmento. Isso mostra que o design considera ambientes de contêiner.

## Por quê

- **Latência vs. vazão**. O BOTR lista o objetivo "GCs should happen as infrequently as
  possible... Each GC should be fast... tune itself". WKS minimiza overhead de threads e
  memória (apps de cliente, ferramentas, processos pequenos). SVR paraleliza o trabalho de GC
  (mark/plan/relocate/compact) para reduzir pausas em máquinas com muitos núcleos, com a
  vantagem de cada heap ser pequena e local ao seu núcleo. **[inferido]** a partir da
  estrutura (um heap/thread por CPU e work stealing).
- **Concorrente por padrão no WKS**: apps interativos preferem pausas curtas de gen2.
- **Gen0 maior no Server**: com mais threads alocando, o orçamento maior reduz a frequência
  de suspensões; mas o limite de 1/6 da RAM evita estouro.
- **DATAS**: contêineres e cargas variáveis fazem um número fixo de heaps desperdiçar
  memória; daí a adaptação.

## Trade-offs

| | Workstation | Server |
|---|---|---|
| Threads de GC | Usa a thread do usuário (e 1 thread BGC se concorrente) | N threads dedicadas (uma por heap) |
| Memória | Menor | Maior (heaps/gen0 por CPU) |
| Pausa (heap grande) | Maior, GC serial | Menor, GC paralelo |
| Afinidade/CPU | Nenhuma | Pode fixar threads a CPUs (`GCHeapAffinitize*`) |
| Complexidade do código | Base | `MULTIPLE_HEAPS`, sincronização entre heaps, join/barreiras, work stealing |
| Convivência com o sistema | Mais amigável a outros processos | Pode monopolizar CPU durante GC |
| Contêineres | Mais previsível | Precisa de `HeapCount`/hard limit/DATAS para não desperdiçar |

Custo do design de dupla compilação: toda alteração deve funcionar em dois modos; o código do
`gc.cpp` está cheio de `#ifdef MULTIPLE_HEAPS` (veja `PER_HEAP_FIELD*` em `gcpriv.h`).

## Como aplicar na sua linguagem

1. **Comece só com o equivalente a Workstation** (um heap, GC na thread que estourou o
   orçamento, stop-the-world). É o mínimo viável e o que o BOTR descreve primeiro.
2. **Prepare a estrutura de dados para múltiplos heaps** mesmo que só use um: marque
   campos como "por heap" vs. "globais" (o .NET usa as macros `PER_HEAP_FIELD`,
   `PER_HEAP_ISOLATED_FIELD`). Isso evita um refactor traumático quando precisar de
   paralelismo.
3. **Paralelize o mark primeiro** (é onde o trabalho é irregular): um heap/região por thread
   de GC e *work stealing* de pilhas de mark.
4. **Torne o número de heaps/threads configurável e com teto por memória** (hard limit,
   percentual). Em contêineres isso decide se a linguagem é viável.
5. **Planeje GC concorrente/background como evolução**, não como requisito inicial: ele
   exige write-watch/barrier (arquivo 03) e muda o protocolo de suspensão.
6. **Meça o tamanho do cache e o use para dimensionar o espaço jovem** (gen0), como em
   `GetCacheSizePerLogicalCpu`.
7. **Exponha dois ou três perfis** (ex.: `latency`, `throughput`, `memory`) mapeando para
   combinações de parâmetros, em vez de dezenas de botões; o .NET, historicamente, expôs vários
   (`gcconfig.h` tem dezenas) e isso é uma dívida de documentação.
8. **Separe GC por interface** (`gcinterface.h`): o CoreCLR consegue carregar um GC
   independente (`docs/design/features/standalone-gc-loading.md`). Para uma linguagem nova,
   isolar o GC permite testar variantes sem tocar no compilador.

## Referências

- `docs/design/coreclr/botr/garbage-collection.md` - "Code Flow", WKS/SVR com e sem GC
  concorrente.
- `src/coreclr/gc/gcconfig.h` - todas as opções citadas (linhas 66-146).
- `src/coreclr/gc/init.cpp` - `get_gen0_min_size`, ajuste por hard limit, modo
  `dynamic_adaptation_to_application_sizes`.
- `src/coreclr/gc/dynamic_heap_count.cpp` - heap count dinâmico (DATAS).
- `src/coreclr/gc/gcpriv.h` - `gc_dynamic_adaptation_mode` (linha 600), macros `PER_HEAP_*`.
- `src/coreclr/gc/mark_phase.cpp` - `mark_steal`.
- `src/coreclr/gc/background.cpp` - thread `.NET BGC`.
- `src/coreclr/gc/finalization.cpp` - namespaces `SVR`/`WKS`, mescla/divisão de filas.
- `docs/design/features/standalone-gc-loading.md`, `standalone-gc-eventing.md`.
