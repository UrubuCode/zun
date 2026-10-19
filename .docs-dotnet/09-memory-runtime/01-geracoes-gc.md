# Gerações do GC (gen0/gen1/gen2) e a hipótese geracional

> Escopo: como o GC do CoreCLR divide o heap em gerações, o que cada geração significa
> fisicamente, e quais decisões de projeto valem para uma nova linguagem com GC.
> Convenção deste documento: **[confirmado]** = lido no código/documentação do repositório;
> **[inferido]** = interpretação minha, não afirmada explicitamente pelo código.

## Mecanismo no .NET

### Visão lógica

O GC do CLR é um coletor **geracional**. Segundo `docs/design/coreclr/botr/garbage-collection.md`
(seção "Logical representation of the managed heap"):

- Objetos pequenos (< 85.000 bytes) vivem em 3 gerações: **gen0, gen1, gen2**.
- Objetos grandes (>= 85.000 bytes) têm **uma única geração** própria (chamada gen3 no
  documento BOTR; no código é o LOH).
- gen0 e gen1 são chamadas de **efêmeras** (ephemeral): objetos de vida curta.
- Quando a geração *N* é coletada, os sobreviventes passam a pertencer à geração *N+1*. Isso
  é a **promoção**. Existem exceções: o GC pode decidir **não promover** ou **rebaixar**
  (demote) objetos.
- "Coletar uma geração" significa coletar essa geração **e todas as mais jovens**.
- Toda alocação acontece na geração mais jovem: gen0 para objetos pequenos, o heap de objetos
  grandes para os grandes.

O mesmo documento ressalva que o número da geração é a idade apenas em princípio: "isso não
significa que todos os objetos em gen0 são mais novos que todos os de gen1 ou gen2" - há
exceções (por exemplo, objetos fixados/pinned que impedem a compactação; ver arquivo 10).

### Constantes no código

Em `src/coreclr/gc/gc.h` (linhas ~114-128) o enum de gerações define:

| Nome no código          | Significado                                              |
|-------------------------|----------------------------------------------------------|
| `max_generation = soh_gen2` | gen2, a geração mais velha do heap de objetos pequenos |
| `loh_generation = 3`    | Large Object Heap                                         |
| `poh_generation = 4`    | Pinned Object Heap                                        |
| `uoh_start_generation = loh_generation` | primeira geração "UOH" (user old heap)      |
| `ephemeral_generation_count = max_generation` | gen0 e gen1                         |
| `total_generation_count = poh_generation + 1` | 5 "gerações" no total              |

Ou seja, o GC atual trata LOH e POH como *gerações adicionais* que só são coletadas junto com
gen2 (documento BOTR: "they are always collected with gen2 collections").

O limite de 85.000 bytes é `LARGE_OBJECT_SIZE` (`src/coreclr/gc/gc.h:106`) e pode ser mudado
por `GCLOHThreshold` / `System.GC.LOHThreshold` (`src/coreclr/gc/gcconfig.h:82`). Em
`interface.cpp:365` o valor configurado nunca fica abaixo do padrão
(`loh_size_threshold = max(loh_size_threshold, LARGE_OBJECT_SIZE)`).

### Visão física

O BOTR descreve o heap como um conjunto de **segmentos** contíguos obtidos do SO, com exatamente
um segmento efêmero onde vivem gen0 e gen1 (e possivelmente parte da gen2). Cada segmento é
consumido de endereços baixos para altos, então, por padrão, endereços menores são mais velhos.

Atenção: o código atual possui o modo **regions** (`USE_REGIONS`, visível em
`src/coreclr/gc/regions_segments.cpp`, `region_allocator.cpp`, `region_free_list.cpp`). O texto
do BOTR ainda fala em segmentos. O documento `docs/design/coreclr/jit/GC-write-barriers.md` já
descreve a variante com tabela `g_region_to_generation_table`, isto é, a geração de uma
região é consultada por tabela. **[confirmado]** que ambos os modos coexistem no código-fonte;
não verifiquei qual está ativo por padrão em cada plataforma.

### Orçamento de alocação (allocation budget)

Cada geração tem um **orçamento**: o tamanho de alocação que, quando excedido, dispara um GC
daquela geração (BOTR, "The allocation budget"). O orçamento é ajustado principalmente pela
**taxa de sobrevivência**: se a geração sobrevive muito, o orçamento cresce, na expectativa de
uma razão morto/vivo melhor na próxima coleta.

A implementação está em `src/coreclr/gc/dynamic_tuning.cpp`: funções `desired_new_allocation`
(linha ~1815) e `surv_to_growth` (~1787) calculam o novo orçamento a partir da sobrevivência.

O tamanho mínimo do orçamento da gen0 vem de `gc_heap::get_gen0_min_size()` em
`src/coreclr/gc/init.cpp` (~1158):

- Workstation: `max(4 * cache_por_cpu_logica / 5, 256 KB)`.
- Server: `max(cache_por_cpu_logica, 256 KB)` (comentário no código: "reduzir pela metade
  parece dar a melhor performance").
- Se `gen0size * n_heaps > memória_física / 6`, reduz pela metade até caber (mas não abaixo do
  tamanho de cache).
- Sempre limitado a metade do tamanho do segmento; com `heap_hard_limit`, a um oitavo.
- Configurável por `GCgen0size`; teto por `GCgen0MaxBudget` (`gcconfig.h:87,102`).

Ponto-chave: **o tamanho da gen0 é amarrado ao tamanho do cache da CPU**. O objetivo é que o
conjunto de trabalho dos objetos jovens caiba em cache. **[inferido]** pela dependência
explícita de `GetCacheSizePerLogicalCpu`.

### Escolha da geração a coletar

Quando um GC é disparado, o coletor decide qual geração condenar. O BOTR lista os fatores além
do orçamento:

1. Fragmentação da geração (alta fragmentação torna a coleta produtiva).
2. Carga de memória da máquina (coletar mais agressivamente para evitar paginação).
3. Pouco espaço no segmento efêmero (mais gen1 para evitar adquirir novo segmento).

Código: `generation_to_condemn()` (chamada em `garbage_collect()` conforme o fluxo descrito no
BOTR) e `GarbageCollectGeneration()`.

### Alocação: contextos e quantum

O alocador entrega memória em **allocation contexts** por thread, preenchidos em unidades de
**allocation quantum** (tipicamente 8 KB, BOTR). Dentro de um contexto a alocação é "bump
pointer" sem lock, porque só uma thread escreve nele. A memória é zerada por quantum (também
para aquecer o cache) e o resto do quantum vira um "free object" para manter o heap *crawlable*
(percorrível linearmente). Objetos grandes não usam contextos e vão direto para um segmento.

### Hipótese geracional no código: cartões

Para coletar só gen0/gen1 o GC precisa saber quais objetos velhos apontam para jovens. Isso é
resolvido por **card table** preenchida por *write barrier* (arquivo 03). O BOTR resume: "Cards
are set by JIT helpers during assignment operations... During ephemeral collections, the GC can
look at the set cards for the rest of the heap and only look at the objects that these cards
correspond to."

## Por quê

A hipótese geracional (empírica) diz que a maioria dos objetos morre jovem. O BOTR enuncia o
objetivo como: coletas devem ser *produtivas* (recuperar bastante), *rápidas* (muitas cargas têm
requisitos de latência) e o custo das coletas efêmeras precisa ser **limitado** ("collecting
ephemeral generations (gen0 and gen1) needs to have a bounded cost" - BOTR).

Consequências de projeto confirmadas no código:

- Alocar sempre em gen0 com bump pointer faz alocação custar quase como incrementar um ponteiro.
- Coletar gen0 custa proporcional à *sobrevivência* (e aos cartões sujos), não ao tamanho do
  heap.
- Duas gerações efêmeras (gen0 e gen1) em vez de uma: gen1 atua como filtro/buffer, evitando
  que objetos de vida média sejam promovidos para gen2 prematuramente. **[inferido]** - o
  BOTR não declara essa motivação textualmente.
- Objetos grandes fora do ciclo efêmero: compactar objeto grande é caro (BOTR), então LOH só é
  varrido em coletas de gen2.

## Trade-offs

| Decisão | Benefício | Custo |
|---|---|---|
| Gerações + cartões | Pausas efêmeras curtas | Write barrier em toda escrita de referência (arquivo 03) |
| Promoção por sobrevivência a uma coleta | Simples, barato | Objeto de vida média pode ser promovido e virar "lixo de gen2" até uma coleta completa |
| Orçamento adaptativo por sobrevivência | Auto-ajuste; programador "não precisa saber do GC" (objetivo do BOTR) | Comportamento menos previsível; muitos parâmetros (`gcconfig.h`) |
| Gen0 do tamanho do cache | Localidade | Dependência da plataforma; limites por memória física |
| LOH separado, coletado com gen2 | Evita copiar objetos grandes | Fragmentação do LOH; alocações grandes de vida curta podem forçar gen2 **[inferido]** |
| Demoção/não-promoção como exceção | Lida com pinning | Invariante "geração = idade" deixa de valer; mais código no plan phase |
| Duas implementações (WKS/SVR) do mesmo código | Reuso | Compilação dupla via `namespace WKS/SVR` (visível em `finalization.cpp`) |

## Como aplicar na sua linguagem

1. **Decida cedo se vai ter GC com movimento.** Gerações com bump allocation exigem poder
   mover (compactar/copiar). Isso condiciona FFI, ponteiros crus e pinning (arquivos 03, 10).
2. **Comece com 2 gerações (jovem/velho)**, como protótipo; o .NET usa 3 (gen0, gen1, gen2)
   mais heaps especiais. Adicionar a terceira depois é uma mudança de política, não de
   arquitetura, *desde que* o write barrier já compare geração do destino com a da origem
   (veja o pseudo-código em `GC-write-barriers.md`, que usa `reg_loc_dst`/`reg_loc_ref`).
3. **Separe objetos grandes** com um limiar configurável (o .NET usa 85.000 bytes). Não os
   copie; trate-os como uma "geração velha" desde o nascimento.
4. **Aloque por thread em blocos** (quantum ~8 KB) para evitar locks no caminho comum; zere em
   bloco.
5. **Use o orçamento como gatilho**, não o heap cheio. Ajuste por sobrevivência observada:
   alta sobrevivência => orçamento maior; baixa => menor.
6. **Mantenha o heap percorrível**: sobras de blocos viram objetos "livres" com cabeçalho
   válido, para que o GC possa varrer linearmente (técnica do BOTR).
7. **Exponha poucos botões**, mas projete a interface GC/runtime separada: o CoreCLR isola o
   GC atrás de `src/coreclr/gc/gcinterface.h` e `gcinterface.ee.h` (permite GC
   substituível - ver `docs/design/features/standalone-gc-loading.md`).
8. **Planeje a telemetria**: eventos de GC (`gcevents.h`, `standalone-gc-eventing.md`) foram
   fundamentais para ajustar heurísticas.

Esboço mínimo de política (pseudocódigo, não é código do .NET):

```
alloc(n):
    if n >= LIMIAR_GRANDE:  return heap_grande.alloc(n)      # coletado só com a geração velha
    if ctx.restante < n:    ctx = novo_quantum(gen0)         # pode disparar GC se orçamento estourou
    return ctx.bump(n)

ao_estourar_orcamento(gen0):
    g = escolher_geracao(fragmentação, carga_memoria, orçamentos)
    coletar(g e todas as mais jovens)
    sobreviventes de g -> geração g+1
    orçamento[g] = f(sobrevivência[g])
```

## Referências

- `docs/design/coreclr/botr/garbage-collection.md` (Maoni Stephens, 2015) - arquitetura,
  gerações, orçamento, fluxo de GC.
- `src/coreclr/gc/gc.h` - enum de gerações, `LARGE_OBJECT_SIZE`.
- `src/coreclr/gc/gcconfig.h` - `GCgen0size`, `GCgen0MaxBudget`, `GCgen1MaxBudget`,
  `GCLOHThreshold`, `GCRegionSize`, `GCHeapHardLimit`.
- `src/coreclr/gc/init.cpp` - `get_gen0_min_size()`.
- `src/coreclr/gc/dynamic_tuning.cpp` - `desired_new_allocation`, `surv_to_growth`.
- `src/coreclr/gc/gcpriv.h`, `gcinternal.h` - estruturas internas (`dynamic_data`, `generation`).
- `src/coreclr/gc/regions_segments.cpp`, `region_allocator.cpp` - modo regions.
- `docs/design/coreclr/jit/GC-write-barriers.md` - barreira com consulta de geração por região.
- Livros citados pelo BOTR: *The Garbage Collection Handbook*; *Pro .NET Memory Management*.
