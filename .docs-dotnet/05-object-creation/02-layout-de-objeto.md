# Layout de objeto: object header word, ponteiro de MethodTable, sync block

## Mecanismo no .NET

### Visão geral

Todo objeto de referência no CLR (CoreCLR) tem esta forma na memória:

```
endereço da referência -->  +------------------+
                            | MethodTable*     |  (m_pMethTab, 1 palavra)
                            +------------------+
                            | campos...        |
                            +------------------+
 referência - sizeof(ptr) -> ObjHeader (m_SyncBlockValue), 1 palavra ANTES do ponteiro
```

Pontos confirmados em `src/coreclr/vm/object.h` e `src/coreclr/vm/syncblk.h`:

- `class Object` (object.h, ~linha 123) tem como único campo declarado
  `PTR_MethodTable m_pMethTab`. Constante `OBJECT_SIZE = TARGET_POINTER_SIZE` (linha 89) —
  o tamanho do "object" vazio sem header é um ponteiro.
- Arrays: `ARRAYBASE_SIZE = OBJECT_SIZE + sizeof(DWORD) /*m_NumComponents*/` (+ um DWORD de
  padding em 64 bits) — linhas 93-95. Ou seja, o comprimento do array fica logo depois do
  ponteiro de MethodTable.
- O **ObjHeader** vive em offset negativo (antes do endereço da referência). Comentário em
  `syncblk.h` (~linha 60): o runtime depende da alocação anterior ter zerado o ObjHeader do
  objeto seguinte, e "os limites do espaço do GC são inicializados para respeitar este erro
  por um" ("off by one"). `SIZEOF_OBJHEADER` é 8 em 64 bits (syncblk.h ~linha 108).
  Ou seja, uma referência aponta para o MethodTable*, não para o início da memória do
  objeto.
- O bit baixo(s) do `m_pMethTab` são usados pelo GC durante a coleta como marca
  (`MARKED_BIT`, object.h ~linhas 170-177, e máscaras `~7`/`~3` nas linhas ~357-359); fora do
  GC o ponteiro é limpo.

### O header word (`m_SyncBlockValue`)

Definido em `src/coreclr/vm/syncblk.h` (linhas ~60-105). O mesmo DWORD tem vários usos
mutuamente exclusivos:

Bits altos (flags):
- `BIT_SBLK_FINALIZER_RUN = 0x40000000` — finalizador já rodou (ou foi suprimido).
- `BIT_SBLK_GC_RESERVE = 0x20000000` — reservado ao GC.
- `BIT_SBLK_SPIN_LOCK = 0x10000000` — spin lock tomado só para modificar o índice.
- `BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX = 0x08000000` — define como interpretar o restante.
- `BIT_SBLK_UNUSED = 0x80000000` (ou `BIT_SBLK_BRIDGE_PENDING` com FEATURE_JAVAMARSHAL).

Se `IS_HASH_OR_SYNCBLKINDEX` está **limpo** (thin lock):
- bits 0-15: id da thread dona (`SBLK_MASK_LOCK_THREADID = 0x0000FFFF`);
- bits 16-21: nível de recursão (`SBLK_MASK_LOCK_RECLEVEL = 0x003F0000`, 64 níveis).

Se **setado**:
- com `BIT_SBLK_IS_HASHCODE = 0x04000000` também setado: os 26 bits baixos são o **hash
  code** (`HASHCODE_BITS = 26`);
- senão: os 26 bits baixos são o **índice do SyncBlock** (`SYNCBLOCKINDEX_BITS = 26`).

Logo o header é um campo "multiuso": nada é pago por objetos que nunca são usados em `lock`
nem têm `GetHashCode` chamado — o header fica 0.

### SyncBlock

Quando uma thin lock não basta (contenção, `Monitor.Wait/Pulse`, recursão > 64, ou objeto
com hash e que precisa de lock simultaneamente, etc.), o runtime aloca uma entrada na tabela
de `SyncBlock`s (`class SyncBlock` em `syncblk.h`; `friend class ObjHeader` linha ~331;
`class ObjHeader` linha ~677) e grava seu índice no header. O SyncBlock guarda o Monitor
completo, o hash e dados de interop (COM/ponteiros), e é recuperado quando o objeto morre.

### MethodTable

`src/coreclr/vm/methodtable.h` descreve a estrutura que o ponteiro referencia: tamanho base
da instância, tabela de métodos virtuais, mapa de interfaces, ponteiro para `EEClass`
(dados frios), flags (finalizável, contém referências, `beforefieldinit` no EEClass), e,
para tipos com referências, o **GCDesc** (`src/coreclr/gc/gcdesc.h`) que descreve onde estão
os ponteiros para o GC percorrer o objeto. Veja também
`docs/design/coreclr/botr/type-system.md`, `method-descriptor.md`.

### Tamanho mínimo e alinhamento

O GC exige tamanho mínimo de objeto (para poder escrever um objeto "livre" no lugar); campos
são ordenados pelo carregador de tipos (auto layout para classes: referências agrupadas,
campos maiores primeiro — comportamento do class loader, não reproduzido aqui em detalhe).
Em 64 bits, objetos são alinhados a 8 bytes; `RhpNewFastAlign8` /
`AllocAlign8` (gcpriv.h) existem para alinhar doubles em 32 bits.

## Por quê

- **Ponteiro de tipo no objeto** habilita despacho virtual, `is`/`as`, cast e reflexão em uma
  única carga; também o GC acha o `GCDesc`/tamanho sem tabelas externas.
- **Header em offset negativo**: a referência aponta ao MethodTable, então o acesso mais
  comum (despacho virtual) não soma offset. Locks/hash são raros e ficam atrás.
- **Header multiuso (thin lock/hash/syncblock index)**: custo zero em memória para o caso
  comum; operações atômicas (CAS) no header dão thin locks sem alocar estruturas.
- **Hash no header**: `RuntimeHelpers.GetHashCode` estável sem campo extra, mesmo com GC
  movendo objetos (endereço não serve como hash).
- **Bits de flag (finalizer run)**: permite `GC.SuppressFinalize` e `ReRegisterForFinalize`
  sem estrutura auxiliar por objeto.

## Trade-offs

- Cada objeto paga **16 bytes de overhead** em 64 bits (header 8 + MethodTable 8), mesmo um
  `object` vazio. Estruturas pequenas (ex.: nós de lista) sofrem.
- Hash e lock disputam o mesmo espaço: um objeto com hash code calculado e depois usado em
  lock precisa inflar para SyncBlock (comportamento da lógica de ObjHeader).
- Thin lock limita o id de thread a 16 bits e recursão a 64 níveis antes de inflar.
- Qualquer objeto pode ser usado em `lock` — decisão de design que obriga a todos pagar o
  header; o .NET 9 introduziu `System.Threading.Lock` dedicado para evitar isso (existe em
  `src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs`).
- Marcar bits do ponteiro de MethodTable durante o GC acopla layout e coletor.

## Como aplicar na sua linguagem

1. Decida o **cabeçalho mínimo**: 1 palavra (ponteiro de tipo ou índice de tipo de 32 bits
   compactado com flags) é o menor razoável para linguagem com GC e despacho virtual.
   Considere comprimir o ponteiro de tipo em 32 bits para reduzir 8 bytes por objeto.
2. Decida se **qualquer objeto** é lockável. Se não, remova a palavra de sync e ofereça um
   tipo `Lock` explícito (como o .NET moderno faz) — economiza memória.
3. Se mantiver hash de identidade, reserve bits do header ou use tabela lateral (identity
   hash map) preenchida sob demanda; evite usar endereço se o GC move objetos.
4. Mantenha flags de GC (marca, forwarded) fora do campo de tipo ou documente claramente a
   máscara — o .NET usa os bits baixos do MethodTable durante a coleta.
5. Gere para cada tipo um **descritor de ponteiros** (equivalente ao GCDesc) em tempo de
   compilação, para o GC não precisar de reflexão para varrer objetos.
6. Para arrays, coloque o comprimento imediatamente após o ponteiro de tipo, como o CLR
   (`ARRAYBASE_SIZE`), para acesso e checagem de limites baratos.
7. Garanta que o header do próximo objeto esteja zerado na alocação (zerar o bloco inteiro
   do allocation context resolve; é o contrato descrito no comentário de `syncblk.h`).

## Detalhes adicionais

### Diagrama de um objeto com campos (64 bits)

```
offset  -8 : ObjHeader (m_SyncBlockValue)                           <- negativo
offset   0 : MethodTable*                                            <- a referência aponta aqui
offset   8 : campo 1 (ex.: ref string)
offset  16 : campo 2 (ex.: long)
...
```

Um objeto sem campos ocupa 16 bytes (header + MethodTable) no mínimo; o GC impõe tamanho
mínimo para poder escrever objetos "livres" no lugar (ver `min_obj_size` em
`src/coreclr/gc/gcpriv.h`).

### Strings e arrays

- `T[]`: header + MethodTable + `m_NumComponents` (+ pad em 64 bits) + elementos; os
  elementos começam em `ARRAYBASE_SIZE` (object.h).
- `string`: segue o mesmo princípio (comprimento inline após o MethodTable e caracteres
  UTF-16 inline); alocada de uma vez com tamanho variável.
- Boxing de struct: header + MethodTable + dados do struct.

### Como hash code e lock interagem (resumo prático)

1. Objeto novo: header = 0.
2. `lock(obj)` sem contenção: CAS grava o id da thread nos 16 bits baixos (thin lock).
3. `GetHashCode()` sem lock: grava o hash nos 26 bits baixos com os bits
   `BIT_SBLK_IS_HASH_OR_SYNCBLKINDEX | BIT_SBLK_IS_HASHCODE`.
4. Contenção, espera ou hash + lock simultâneos: o runtime cria um SyncBlock e grava o
   índice no header.

Este resumo deriva da disposição de bits em `syncblk.h`; os algoritmos de inflação ficam em
`syncblk.cpp` (mesmo diretório; não detalhados aqui).

### Variante NativeAOT

O NativeAOT usa o mesmo conceito de ponteiro de tipo + header, com runtime próprio em
`src/coreclr/nativeaot/Runtime/`. Isso mostra que o desenho é replicável em runtimes
menores.

## Referências

- `src/coreclr/vm/object.h` — `class Object`, `OBJECT_SIZE`, `ARRAYBASE_SIZE`, `MARKED_BIT`.
- `src/coreclr/vm/syncblk.h` — `BIT_SBLK_*`, `SBLK_MASK_*`, `SyncBlock`, `ObjHeader`.
- `src/coreclr/vm/methodtable.h` — estrutura de MethodTable.
- `src/coreclr/gc/gcdesc.h` — descritor de referências para o GC.
- `docs/design/coreclr/botr/type-system.md`, `method-descriptor.md`,
  `garbage-collection.md`.
- `src/libraries/System.Private.CoreLib/src/System/Threading/Lock.cs`.
