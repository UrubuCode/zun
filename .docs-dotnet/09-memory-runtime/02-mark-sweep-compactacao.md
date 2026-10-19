# Mark-and-sweep e compactação

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.

## Mecanismo no .NET

O GC do CLR **não** é um simples mark-and-sweep: ele é um coletor *mark / plan / (relocate +
compact | sweep)*. A descrição canônica está em `docs/design/coreclr/botr/garbage-collection.md`
(seção "The flow of a GC") e cada fase tem arquivo próprio em `src/coreclr/gc/`:

| Fase | O que faz (BOTR) | Arquivo |
|---|---|---|
| Mark | Encontra todos os objetos vivos | `mark_phase.cpp` (3971 linhas) |
| Plan | "Simula" uma compactação e decide: compactar ou varrer | `plan_phase.cpp` (8445 linhas) |
| Relocate | Atualiza todas as referências para objetos que vão se mover | `relocate_compact.cpp` |
| Compact | Copia os objetos para os endereços calculados no plan | `relocate_compact.cpp` |
| Sweep | Transforma o espaço morto entre objetos vivos em "free objects" na freelist | `sweep.cpp` |

### Fluxo de controle (modo não concorrente)

Do BOTR (seção "Physical Architecture"), em pseudocódigo fiel ao documento:

```
GarbageCollectGeneration() { SuspendEE(); garbage_collect(); RestartEE(); }
garbage_collect()          { generation_to_condemn(); gc1(); }
gc1()                      { mark_phase(); plan_phase(); }
plan_phase()               { /* decide */ if (compact) { relocate_phase(); compact_phase(); }
                             else make_free_lists(); }
```

Todas as threads gerenciadas são suspensas (`SuspendEE`) durante essas fases (exceto no GC em
background, ver abaixo).

### Mark

Objetivo: achar os vivos. As **raízes** vêm do Execution Engine (pilhas, handles, estáticos)
e, em coletas efêmeras, também dos objetos de **gerações mais velhas que apontam para jovens**
- encontrados via *cards* (arquivo 03). O BOTR observa que a fase de mark "só consulta objetos
vivos" e por isso não precisa considerar referências fracas.

Detalhes confirmados em `src/coreclr/gc/mark_phase.cpp`:

- O percurso do grafo é **iterativo com pilha explícita**: `mark_object_simple1` mantém
  `mark_stack_array` / `mark_stack_tos` / `mark_stack_limit` (linha ~1731). Não usa recursão
  da pilha nativa. **[confirmado]**
- Em GC Server há **work stealing** entre as threads de heap: `gc_heap::mark_steal()`
  (linha ~1970) chama `mark_object_simple1` para objetos roubados de outro heap.
- `mark_phase(int condemned_gen_number)` zera contadores por geração (`dd_survived_size`,
  `dd_pinned_survived_size`, etc.) antes de marcar.
- O layout dos objetos é descrito por *GC descriptors* (`src/coreclr/gc/gcdesc.h`), de modo que
  o GC não precisa de tratamento por tipo: "The GC does not have special treatment for
  different kinds of object types. It consults the EE to get the size of an object." (BOTR).

### Plan: a decisão compactar vs. varrer

O plan percorre o heap condenado agrupando sobreviventes contíguos em **plugs** (execuções de
objetos vivos) separados por **gaps** (mortos), e calcula para onde cada plug iria se houvesse
compactação. **[confirmado]** que o código trabalha com "plugs" (`pinned_plug`, `plug`,
`relocate_survivors_in_plug`, `relocate_survivors_in_brick(uint8_t* tree, ...)` - a "brick
table" indexa árvores de plugs por brick; propósito exato **[inferido]** a partir dos nomes
e do uso).

A decisão final é `gc_heap::decide_on_compacting` (`plan_phase.cpp:7933`). Lendo a função,
forçam compactação (entre outras):

- `GCConfig::GetForceCompact()`;
- última coleta antes de OOM em gen2 (`last_gc_before_oom`);
- razões induzidas: `reason_induced_compacting`, `reason_induced_aggressive`,
  `reason_pm_full_gc`;
- gen1 em "provisional mode" (`provisional_mode_triggered`);
- regions: `decide_on_compaction_space()`; segments: pouco espaço efêmero
  (`dt_low_ephemeral_space_p`);
- **fragmentação**: `fragmentation >= dd_fragmentation_limit(dd)` **e**
  `fragmentation_burden >= dd_fragmentation_burden_limit(dd)`, onde
  `fragmentation_burden = fragmentation / gen_sizes`;
- memória física alta: acima de `high_memory_load_th` compacta se o espaço recuperável
  (`generation_size(max_generation) - generation_plan_size(max_generation)`) passa de um
  limite que depende do espaço físico disponível e do nº de heaps.

E antes disso, em `plan_phase.cpp:~4485`, para 64 bits: se a coleta é efêmera e
`gen0_reduction_count > 0` ou a carga de memória na entrada é >= 95%, compacta.

Em coletas de gen2 o LOH é varrido (`sweep_uoh_objects(loh_generation)`) a menos que
`settings.loh_compaction` esteja ligado (`FEATURE_LOH_COMPACTION`, função `compact_loh()` em
`relocate_compact.cpp:87`). POH é sempre varrido ali.

### Sweep e freelist

Sweep "procura o espaço morto entre objetos vivos. Cria free objects no lugar. Objetos mortos
adjacentes viram um só free object. Coloca todos na freelist" (BOTR). O alocador depois
reutiliza esses buracos para objetos que não cabem no bump de gen0 normal. O tamanho mínimo
de item na freelist é `min_free_list = 2*min_obj_size` (`gcpriv.h:6101`).

Há também "sweep in plan" para regions (`sweep_region_in_plan`, `plan_phase.cpp:5936`;
`special_sweep_p`), em que o plan já varre a região em vez de planejar movimentação.

### Compactação e relocação

Como o plan já calculou o novo endereço, o compact "é bem direto": copia. O trabalho difícil
é o **relocate**: percorrer *todas* as referências para objetos das gerações condenadas -
inclusive as vindas de gerações velhas (via cards), de handles e da pilha - e reescrevê-las
(`relocate_address`, `relocate_survivors`, `relocate_survivors_in_plug`;
`relocate_compact.cpp`). Isso também vale para a fila de finalização
(`CFinalize::RelocateFinalizationData`, arquivo 11).

### GC em background (concorrente)

Com `gcConcurrent` ligado (padrão `true`, `gcconfig.h:67`), uma coleta de gen2 pode ser feita
por uma thread de background: o mark roda **enquanto as threads de usuário continuam**, e o
sweep também (`background_mark_phase` / `background_sweep` em `background.cpp`).
O BOTR detalha: GC efêmero (foreground) pode ocorrer *durante* o background GC. Background GC
**só varre**; não compacta. **[inferido]** pelo fluxo `background_mark_phase(); background_sweep();`
do BOTR, que não tem relocate/compact.
Para rastrear mutações concorrentes o GC usa *write watch* (hardware ou software, ver
`softwarewritewatch.cpp` e a operação `SwitchToWriteWatch` do write barrier, arquivo 03).

## Por quê

- **Mark-and-sweep puro** é barato por coleta, mas deixa o heap fragmentado e obriga alocação
  por freelist (lenta, ruim para cache). **Compactar** restaura alocação por bump pointer e
  localidade, mas exige mover objetos e atualizar referências (custo proporcional aos
  sobreviventes).
- A decisão **adiada para depois do mark** (plan) permite usar informação real: quanto
  sobreviveu e quão fragmentado ficou. O BOTR diz que se a compactação for produtiva o GC
  compacta, senão varre.
- Os metas do BOTR: coleta *produtiva*, *rápida*, auto-ajustável. Compactar sempre seria
  produtivo em layout mas caro em CPU; varrer sempre seria o oposto.
- O mark em pilha explícita evita estouro de pilha nativa com grafos profundos (listas
  encadeadas longas). **[inferido]**.

## Trade-offs

| Escolha | Ganho | Perda |
|---|---|---|
| Compactar | Bump allocation, localidade, sem fragmentação | Precisa suspender o mundo; atualiza referências; objetos fixados (pinned) atrapalham |
| Varrer | Sem mover objetos; barato quando sobrevivência é alta | Fragmentação, freelist, alocação mais lenta |
| Plan separado | Decisão informada, reutiliza informação para ambos os caminhos | Fase extra e estruturas auxiliares (plugs, bricks, pinned queue) |
| LOH não compactado por padrão | Evita copiar objetos grandes | Fragmentação do LOH (compactação opt-in) |
| Background GC | Pausas curtas em gen2 | Complexidade enorme (write watch, mark concorrente, foreground durante background); não compacta |
| Heurísticas por limiares | Funciona bem em média | Muitos limites empíricos (carga de memória 95%, fragmentation limits) difíceis de portar |

## Como aplicar na sua linguagem

1. **Implemente mark com pilha explícita** (ou lista de trabalho) desde o primeiro dia. Para
   muitas threads de GC, considere work stealing como no `mark_steal`.
2. **Separe "decidir" de "executar"**: faça uma fase de *plan* que calcula os endereços novos
   e a fragmentação planejada antes de tocar no heap. Assim você escolhe compactar ou varrer
   com dados.
3. **Escolha uma política por classe de objeto**: compacte o espaço jovem (barato, poucos
   sobreviventes) e permita só varrer o espaço velho/grande, compactando apenas sob
   fragmentação ou pressão de memória - é exatamente o padrão do `decide_on_compacting`.
4. **Projete o relocate como um passe geral sobre todas as "fontes de referência"**
   (pilhas, handles, tabelas auxiliares, fila de finalização, cards). Tudo que guarda um
   ponteiro para o heap precisa ser relocável - ou fixado.
5. **Torne o heap percorrível** (objetos livres com cabeçalho), pois sweep e plan assumem
   caminhada linear.
6. **Concorrência é opcional e cara.** Só adicione GC em background se pausas de gen2 forem
   inaceitáveis; exige write watch ou barreira equivalente (arquivo 03). Para uma primeira
   versão, stop-the-world + compactação opcional é bem mais simples.
7. **Registre o motivo de cada decisão** (o .NET grava mecanismos como `compact_high_frag`,
   `compact_low_ephemeral` em `get_gc_data_per_heap()->set_mechanism`) - ajuda a depurar
   heurísticas.

Esqueleto sugerido:

```
coletar(g):
    parar_mundo()
    marcar(raizes + cartoes_das_geracoes_mais_velhas)          # pilha explicita
    plano = planejar(g)                                         # plugs, gaps, fragmentacao
    se plano.vale_compactar():                                  # fragmentacao, memoria, flags
        relocar_todas_as_referencias(plano); mover(plano)
    senao:
        varrer_para_freelist(plano)
    retomar_mundo()
```

## Referências

- `docs/design/coreclr/botr/garbage-collection.md` - seções "The flow of a GC", "Code Flow",
  "Physical Architecture".
- `src/coreclr/gc/mark_phase.cpp` - `mark_phase`, `mark_object_simple1`, `mark_steal`.
- `src/coreclr/gc/plan_phase.cpp` - `plan_phase` (linha 3315), `decide_on_compacting` (7933),
  `sweep_region_in_plan` (5936).
- `src/coreclr/gc/relocate_compact.cpp` - `relocate_address`, `relocate_survivors*`,
  `compact_loh`.
- `src/coreclr/gc/sweep.cpp` - construção de free lists.
- `src/coreclr/gc/background.cpp` - `background_mark_phase`, `background_sweep`.
- `src/coreclr/gc/gcdesc.h` - descritores de layout para marcação.
- `src/coreclr/gc/gcpriv.h` - `min_free_list`, `dynamic_data`.
