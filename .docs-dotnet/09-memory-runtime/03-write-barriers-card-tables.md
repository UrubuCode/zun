# Write barriers e card tables

> **[confirmado]** = lido no repositório. **[inferido]** = interpretação minha.

## Mecanismo no .NET

### O problema

Para coletar apenas gen0/gen1 sem varrer o heap inteiro, o GC precisa saber quais objetos
**velhos** guardam referências para objetos **jovens**. Cada vez que o programa grava uma
referência em um campo/elemento no heap, o compilador emite uma chamada a um helper chamado
**write barrier** que registra essa informação numa **card table**.

O BOTR (`docs/design/coreclr/botr/garbage-collection.md`, "Mark phase") diz: "Cards are set by
JIT helpers during assignment operations. If the JIT helper sees an object in the ephemeral
range it will set the byte that contains the card representing the source location."

O próprio repositório afirma que `JIT_WriteBarrier` é "geralmente a função mais quente do
CoreCLR e é escrita em assembly" (`docs/design/coreclr/jit/GC-write-barriers.md`).

### Estruturas

- **Card**: uma fatia fixa do heap. Em `src/coreclr/gc/gcpriv.h` (linhas ~6760-6771):
  `GC_PAGE_SIZE = 0x1000`, `card_word_width = 32`, e
  `card_size = 2*GC_PAGE_SIZE/card_word_width` em 64 bits (= **256 bytes**) ou
  `GC_PAGE_SIZE/card_word_width` em 32 bits (= 128 bytes). O comentário no código diz que o
  valor foi "determinado empiricamente de acordo com o tamanho médio de um objeto" e que uma
  palavra da card table (`uint32_t`) cobre uma página inteira do SO. **[confirmado]**
- **Card table**: um bit por card no GC, mas o write barrier x64 trabalha com **um byte por
  8 cards** em um dos modos (ver abaixo): `shr rcx, 0Bh` => 2048 bytes de heap por byte da
  tabela (`src/coreclr/vm/amd64/JitHelpers_FastWriteBarriers.asm`).
- **Card bundle table**: um segundo nível, marcando grupos de cards sujos para que o GC
  pule rapidamente grandes regiões limpas. O pseudocódigo do write barrier usa `dst >> 21`
  como índice (`GC-write-barriers.md`). Em `gcinternal.h:2015` aparece
  `card_bundle_size = GC_PAGE_SIZE / (sizeof(uint32_t)*card_bundle_word_width)`.
- **Brick table**: índice auxiliar para achar o início de objetos dentro de um card
  (`brick_table` em `gcpriv.h`; usado por `find_card`, `card_table.cpp`). Detalhe de uso
  **[inferido]**.

### O write barrier (pseudocódigo oficial)

Reproduzido de `docs/design/coreclr/jit/GC-write-barriers.md` (versão com regions):

```
JIT_WriteBarrier(Object **dst, Object *ref)
    *dst = ref                                  # a escrita em si

    if g_sw_ww_table != 0:                      # write watch por software (GC em background)
        ww = g_sw_ww_table + (dst >> 11); if *ww == 0: *ww = 0xff

    if ref < g_ephemeral_low || ref >= g_ephemeral_high:   # ref não é efêmera?
        return

    if g_region_to_generation_table != 0:       # modo regions
        reg_dst = tabela[dst >> g_region_shr];  reg_ref = tabela[ref >> g_region_shr]
        if reg_dst == 0:        return          # destino em gen0 -> não precisa de card
        if reg_ref >= reg_dst:  return          # não é "velho apontando para jovem"
        if g_region_use_bitwise_write_barrier:  # variante em bits (atômica)
            card = (dst >> 11) + g_card_table; bit = 1 << ((dst >> 8) & 7)
            if (*card & bit) == 0: return        # (sic) como no documento
            lock: *card |= bit ;  goto CardBundle

    card = (dst >> 11) + g_card_table
    if *card == 0xff: return                    # já suja: evita escrita (e cache line bouncing)
    *card = 0xff

CardBundle:
    if FEATURE_MANUALLY_MANAGED_CARD_BUNDLES:
        b = (dst >> 21) + g_card_bundle_table;  if *b != 0xff: *b = 0xff
```

Pontos importantes **[confirmados]** no assembly de `JitHelpers_FastWriteBarriers.asm`:

1. A escrita `mov [rcx], rdx` vem **primeiro**; o comentário diz que "é correto tomar um AV
   aqui": o tratamento de exceções mapeia a falha de volta ao método gerenciado chamador.
2. Há versões `PreGrow64` e `PostGrow64`: "a região efêmera estará sempre no topo do heap";
   enquanto isso vale, só o limite **inferior** é checado. Depois que o GC move a região
   efêmera, usa-se a versão que checa os dois limites.
3. Os limites efêmeros e o endereço da card table são **constantes imediatas dentro do
   código**, remendadas em tempo de execução (labels `..._Patch_Label_Lower`,
   `..._Patch_Label_CardTable`). "É imperativo que os endereços dos valores sobrescritos sejam
   naturalmente alinhados" porque são reescritos com o EE em execução.
4. Antes de escrever na card table, **lê** o byte e só escreve se ainda não for `0xFF`
   (`cmp byte ptr [rcx + rax], 0FFh / jne UpdateCardTable`). Evita tráfego de cache entre
   cores.

### Variantes e o WriteBarrierManager

`src/coreclr/vm/writebarriermanager.h` enumera os tipos, entre eles `WRITE_BARRIER_PREGROW64`,
`POSTGROW64`, `SVR64`, `BYTE_REGIONS64`, `BIT_REGIONS64`, e as correspondentes
`WRITE_BARRIER_WRITE_WATCH_*`. O doc diz: o barrier em uso é uma **cópia** de uma dessas
implementações, em um buffer (`JIT_WriteBarrier` buffer); o `WriteBarrierManager` troca a cópia
quando o estado interno muda (na prática só na entrada/saída do modo write-watch).

A interface GC -> EE para isso é `GCToEEInterface::StompWriteBarrier(WriteBarrierParameters*)`
com a operação `WriteBarrierOp { StompResize, StompEphemeral, Initialize, SwitchToWriteWatch,
SwitchToNonWriteWatch }` (`src/coreclr/gc/gcinterface.h:49`). Ou seja: sempre que o GC
redimensiona a tabela ou move a fronteira efêmera, ele pede ao runtime que **remende o
código** do barrier.

### Barrier "checked"

```
JIT_CheckedWriteBarrier(dst, ref):
    if (ref < g_lowest_address || ref >= g_highest_address) return   # (texto do doc)
    JIT_WriteBarrier(dst, ref)
```

Usado quando o JIT não sabe se o destino está no heap gerenciado (por exemplo, escrita via
byref que pode apontar para pilha ou memória nativa). No JIT:
`GCInfo::gcIsWriteBarrierCandidate` (`src/coreclr/jit/gcinfo.cpp:227`) decide:

| Situação do store | Forma |
|---|---|
| Tipo não é `TYP_REF` (byrefs nunca estão no heap gerenciado como valor) | sem barrier |
| Valor é constante 0 (null) ou handle de objeto não-GC (`GTF_ICON_OBJ_HDL`) | sem barrier |
| `GTF_IND_TGT_NOT_HEAP` (destino sabidamente fora do heap) | sem barrier |
| `GTF_IND_TGT_HEAP` (destino sabidamente no heap) | barrier **unchecked** |
| Desconhecido | barrier **checked** |

Os helpers correspondentes em `src/coreclr/inc/corinfo.h` são `CORINFO_HELP_ASSIGN_REF` e
`CORINFO_HELP_CHECKED_ASSIGN_REF` (e variantes por registrador em x86).

Objetos alocados na pilha (arquivo 09) interagem: segundo `object-stack-allocation.md`, se a
referência sempre aponta para objeto da pilha, nenhum barrier; se pode ser heap ou pilha,
usa-se o barrier *checked*.

### Write watch (suporte a GC em background)

O GC em background precisa saber quais páginas foram modificadas durante o mark concorrente.
O barrier atualiza uma **write-watch table** (`g_sw_ww_table`, 1 byte por 2 KB: `dst>>11`),
implementada por software em `src/coreclr/gc/softwarewritewatch.cpp`, quando ativada via
`SwitchToWriteWatch`.

### Debug

Com `WRITE_BARRIER_CHECK` (apenas DEBUG) existe um "shadow heap" (`g_GCShadow`) para detectar
escritas sem barrier: grava a referência também no shadow, e invalida se o valor real divergir.

## Por quê

- **Custo limitado das coletas efêmeras**: sem cards, gen0 exigiria varrer gen1+gen2 inteiras
  para achar ponteiros "para o futuro". Com cards, só se visitam os objetos dos cards sujos.
- **Barato no caminho feliz**: a maioria das escritas é jovem->jovem, null, ou velho->velho.
  Por isso as primeiras checagens são comparações de intervalo com constantes imediatas
  (sem carregar variáveis globais) - todo o desenho do barrier "PreGrow/PostGrow" existe para
  tirar loads da memória do caminho quente. **[inferido]** a partir dos comentários do .asm.
- **Marcar em bytes inteiros (0xFF)** evita instruções atômicas (`lock or`): duas threads
  escrevendo `0xFF` no mesmo byte não conflitam semanticamente. A variante *bitwise* (regions)
  precisa de `lock or`, e no Arm64 só é usada se há atômicos LSE (`GC-write-barriers.md`).
- **Checar a geração do destino/origem por região** (modo regions) reduz cards falsos:
  só marca quando é de fato "velho -> jovem".

## Trade-offs

| Escolha | Ganho | Custo |
|---|---|---|
| Barrier em todo store de referência | Coletas efêmeras baratas | Overhead em cada escrita; código de barrier é o "hot spot" do runtime |
| Card grande (256 B) | Tabela pequena, barrier simples | Imprecisão: GC varre objetos inteiros do card (falso positivo) |
| Barrier remendado em runtime | Zero loads extras | Código auto-modificável: complicado, depende de alinhamento, de W^X, de plataforma |
| Barrier checked | Segurança com ponteiros desconhecidos | Mais um par de comparações |
| Dois níveis (card + bundle) | Pular rapidamente áreas limpas | Mais uma escrita no caminho lento |
| Write watch por software | Habilita GC concorrente em plataformas sem write-watch por hardware | Custo adicional em toda escrita enquanto ativo |
| Eliminação de barrier no JIT (null, não-heap) | Menos custo | Exige análise de destino e flags corretas - bug = corrupção de heap |

## Como aplicar na sua linguagem

1. **Se tiver GC geracional ou incremental, você terá write barrier.** Desenhe a IR para que
   toda escrita de referência seja uma instrução distinta (como `STOREIND` do tipo `TYP_REF`),
   para que o backend possa classificá-la (nenhum / unchecked / checked).
2. **Defina 3 formas de barrier desde o início** (como `WBF_NoBarrier`, `WBF_BarrierUnchecked`,
   `WBF_BarrierChecked`): cada uma permite a seu otimizador provar mais coisas e economizar.
   Isenções seguras: armazenar `null`; armazenar constantes/objetos imortais (heap congelado,
   ver `NonGC-Heap.md`); escrever em pilha/registradores.
3. **Mantenha o barrier minúsculo e fora de linha**: um helper no runtime que o compilador
   chama com registradores fixos. Compare primeiro com **limites constantes** e só então toque
   na card table.
4. **Escreva primeiro, marque depois**, e **teste antes de escrever** o byte da card
   (evita invalidar cache lines compartilhadas).
5. **Escolha o tamanho do card** equilibrando tamanho da tabela e custo da varredura
   (o .NET: 256 B em 64 bits). Um segundo nível de "bundle" ajuda se o heap for grande.
6. **Se você quer GC concorrente**, planeje a tabela de write-watch/dirty no mesmo barrier e
   uma operação para ligar/desligar (o `SwitchToWriteWatch` do .NET).
7. **Prefira a interface "GC pede ao runtime para ajustar o barrier"** (`StompWriteBarrier`):
   se os limites do heap mudam, o GC só chama a operação; o resto do compilador não sabe.
   Se não quiser remendar código, guarde os limites em globais (um load a mais por escrita) -
   decisão consciente de simplicidade vs. velocidade.
8. **Teste com um "shadow heap"** em modo debug, como `WRITE_BARRIER_CHECK`, para encontrar
   barriers faltando: é o bug mais traiçoeiro de um GC geracional.
9. Considere **interoperabilidade**: `ref`/byrefs podem apontar para pilha, heap ou memória
   nativa, o que leva à necessidade do barrier "checked".

## Referências

- `docs/design/coreclr/jit/GC-write-barriers.md` - pseudocódigo completo, `WriteBarrierManager`,
  nota sobre Arm64/LSE.
- `docs/design/coreclr/botr/garbage-collection.md` - seção "Mark phase" (cards).
- `src/coreclr/vm/amd64/JitHelpers_FastWriteBarriers.asm` (e `jithelpers_fastwritebarriers.S`)
  - implementação x64 com constantes remendáveis.
- `src/coreclr/vm/writebarriermanager.cpp` / `.h` - seleção do barrier.
- `src/coreclr/gc/gcinterface.h` (`WriteBarrierOp`, `WriteBarrierParameters`) e
  `gcinterface.ee.h` (`StompWriteBarrier`).
- `src/coreclr/gc/gcpriv.h` (`card_size`, `card_word_width`), `gcinternal.h` (`card_bundle_size`),
  `src/coreclr/gc/card_table.cpp` (`find_card`, varredura de cards).
- `src/coreclr/gc/softwarewritewatch.cpp` - write watch por software.
- `src/coreclr/jit/gcinfo.cpp` - `gcIsWriteBarrierCandidate`.
- `src/coreclr/inc/corinfo.h` - `CORINFO_HELP_ASSIGN_REF`, `CORINFO_HELP_CHECKED_ASSIGN_REF`.
- `docs/design/coreclr/jit/object-stack-allocation.md` - barrier e objetos na pilha.
