# Design das coleções concorrentes

`ConcurrentDictionary`, `ConcurrentQueue`, `ConcurrentStack`, `ConcurrentBag` e `BlockingCollection`:
quatro estratégias diferentes de sincronização, cada uma escolhida para o padrão de acesso da estrutura.

## Mecanismo no .NET

Localização: `ConcurrentDictionary` e `ConcurrentQueue` (+ `ConcurrentQueueSegment`) ficam em
`src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/` (porque o próprio CoreLib as usa);
`ConcurrentStack`, `ConcurrentBag`, `BlockingCollection`, `Partitioner` em
`src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/`.

### 1. `ConcurrentDictionary<TKey,TValue>`: leitura sem lock, escrita com lock por faixa

Estrutura (`ConcurrentDictionary.cs`):

- `private volatile Tables _tables;` — objeto imutável-na-referência com `_buckets` (array de `VolatileNode`),
  `_locks` (array de `object`), `_countPerLock` (contagem por lock) e `_comparer`. Trocado por uma nova instância a cada redimensionamento.
- **Leituras (`TryGetValue`, `ContainsKey`) são lock-free**: copiam `_tables` para um local, percorrem a
  cadeia do bucket via `Volatile.Read`. Comentário-chave: ler o campo volátil uma vez em um local e usar
  `Volatile.Read` em elementos do array protege de "ler campos de instâncias diferentes".
- **Escritas travam apenas o lock da faixa** (`GetBucketAndLock(tables, hashcode, out lockNo)` →
  `Monitor.Enter(locks[lockNo])`). Isso é *lock striping*. Depois de adquirir, confere `tables != _tables`
  (redimensionou?) e, se sim, recomeça com a nova tabela ("rare occurrence").
- Nós novos entram na cabeça do bucket e são publicados com `Volatile.Write(ref bucket, newNode)`; assim um
  leitor sem lock vê o nó completamente construído (doc 04).
- **Atualização de valor**: se `TValue` é escrita atômica (`IsTValueWriteAtomic`), sobrescreve `node._value`;
  senão **cria um novo nó** com o valor novo e troca o ponteiro — "para suportar tipos TValue que não podem ser
  escritos atomicamente, já que leituras sem lock podem ocorrer concorrentemente" (comentário).
- **Nível de concorrência**: `DefaultConcurrencyLevel => Environment.ProcessorCount`; `DefaultCapacity = 31`
  buckets; `MaxLockNumber = 1024` (teto ao crescer o array de locks automaticamente; o usuário pode excedê-lo
  passando `concurrencyLevel` maior). `_growLockArray` controla se locks crescem junto com a tabela.
- **Crescimento**: `_budget` = máximo de elementos por lock antes de redimensionar. Quando
  `_countPerLock[lockNo] > _budget`, sinaliza `resizeDesired`; a chamada a `GrowTable` acontece **depois de
  soltar o lock** do bucket ("necessário para prevenir deadlocks"). `GrowTable` adquire **todos** os locks
  em ordem (`AcquireAllLocks`), usa tamanhos primos (`HashHelpers.GetPrime`), pode dobrar `_locks`
  (criando novos objetos de lock) e reconstrói a tabela.
- **Operações globais** (`Count`, `Clear`, `ToArray`, `Keys`, `Values`, enumeração de snapshot em alguns
  casos) adquirem todos os locks (`AcquireAllLocks`) — caras e bloqueiam escritores. Já o enumerador padrão
  não trava: percorre os buckets com `Volatile` e não garante snapshot consistente.
- **`GetOrAdd(key, valueFactory)` / `AddOrUpdate`**: o delegate `valueFactory` é chamado **fora do lock**
  (`TryGetValueInternal` primeiro; se ausente, `TryAddInternal(... valueFactory(key) ...)`). Portanto o
  delegate pode ser executado mais de uma vez para a mesma chave sob corrida, e o valor "perdedor" é
  descartado; há overloads com argumento (`TArg`) para evitar closures.
- **Otimização de comparador**: para chaves de valor com comparador padrão, o JIT elimina o ramo do comparador
  (`typeof(TKey).IsValueType && comparer is null`); para classes com comparador default usa
  `key.GetHashCode()` direto (`_comparerIsDefaultForClasses`) para economizar despacho de interface.
- Alternate lookup (`IAlternateEqualityComparer`) tem caminho próprio (`IsCompatibleKey<TAlternateKey>`).

### 2. `ConcurrentQueue<T>`: lista ligada de segmentos circulares lock-free

`ConcurrentQueue.cs` e `ConcurrentQueueSegment.cs`:

- Cada **segmento** é um buffer circular limitado (potência de 2) de `Slot`s `{ Item, SequenceNumber }`.
  `_headAndTail` é `PaddedHeadAndTail` (preenchido para evitar false sharing entre head e tail).
- **Enqueue/Dequeue por números de sequência** (algoritmo estilo Vyukov): o produtor lê `Tail`, olha o
  `SequenceNumber` do slot; se == tail, o slot está livre → CAS em `Tail` → escreve o item e
  `Volatile.Write(slot.SequenceNumber, tail + 1)` publica. O consumidor espera
  `SequenceNumber == head + 1`, faz CAS em `Head`, lê o item e escreve `SequenceNumber = head + length` para
  liberar o slot para a próxima volta. Se o produtor/consumidor "ganhou o CAS mas ainda não publicou",
  outros giram (`SpinWait`) no slot.
- A fila é **ilimitada**: segmentos iniciam com `InitialSegmentLength = 32` e o tamanho cresce até
  `MaxSegmentLength = 1024 * 1024`. Quando o segmento enche, é **congelado para enqueues**
  (`_frozenForEnqueues`, via `Interlocked.Add(ref Tail, FreezeOffset)`) e um novo é ligado; um
  `_crossSegmentLock` protege apenas operações entre segmentos (atualizar `_head`/`_tail`, obter visão consistente).
- **Snapshots** (enumeração, `ToArray`, `Count` cara) marcam segmentos como "preservados para observação"
  (`_preservedForObservation`): dequeues não sobrescrevem os slots, então uma enumeração pode ler dados
  estáveis. Cada elemento enfileirado não aloca nó (usa slots do array).
- `IsEmpty` e `TryPeek` têm laços com `Volatile.Read` e verificações de reconfirmação (head/tail inalterados).

### 3. `ConcurrentStack<T>`: pilha de Treiber

Lista ligada simples com `volatile Node? _head`. `Push`: `newNode._next = _head; CAS(ref _head, newNode,
newNode._next)`; falha → `PushCore` com backoff. `PushRange`/`TryPopRange` operam em lote de forma atômica.

### 4. `ConcurrentBag<T>`: filas locais por thread com work stealing

Documentação: "otimizada para cenários em que a mesma thread produz e consome". Implementação:
`ThreadLocal<WorkStealingQueue> _locals` e lista ligada `volatile WorkStealingQueue? _workStealingQueues`.
`Add` vai à fila da própria thread; `TryTake` tenta `TryLocalPop` e, se vazio, `TrySteal` das outras.
Criar fila para uma thread nova usa `lock (GlobalQueuesLock)` e pode reaproveitar fila "sem dono"
(`GetUnownedWorkStealingQueue`). Congelamento global para operações de snapshot.

### 5. `BlockingCollection<T>`: decorador de bloqueio/limite

Envolve qualquer `IProducerConsumerCollection<T>` (padrão `ConcurrentQueue<T>`) com dois `SemaphoreSlim`:
`_freeNodes` (capacidade restante, só se `boundedCapacity` definida) e `_occupiedNodes` (itens disponíveis).
`Add` espera `_freeNodes`, `Take` espera `_occupiedNodes`; `CompleteAdding` sinaliza término e
`IsCompleted` = adding completo e `_occupiedNodes.CurrentCount == 0`. Bloqueante (síncrono) — para o equivalente
assíncrono veja Channels (doc 10).

### Outras

`Partitioner`/`OrderablePartitioner` (particionamento para `Parallel`/PLINQ),
`IProducerConsumerCollection<T>` (contrato `TryAdd/TryTake`), `CDSCollectionETWBCLProvider` (eventos).

## Por quê

- **Cada estrutura casa com um padrão de acesso**: dicionário (muitas leituras, escritas dispersas por
  chave → striping + leitura lock-free), fila (multi-produtor/consumidor → lock-free por slot), pilha
  (um ponto de contenção → CAS simples), bag (produtor=consumidor → localidade por thread).
- **Leituras são o caso comum** em caches/dicionários: tirar o lock delas reduz drasticamente contenção.
- **Escala com núcleos**: `concurrencyLevel = ProcessorCount`, cache-line padding, segmentos para evitar
  alocar por item.
- **Segurança de redimensionamento**: tabela substituída por referência volátil + checagem pós-lock.
- **Compatibilidade com enumeração segura** (snapshot na fila) sem travar escritores.

## Trade-offs

- `Count`/`ToArray`/`Keys`/`Values` do dicionário travam **tudo** — evitá-los em caminho quente.
- `GetOrAdd(valueFactory)` pode executar o factory várias vezes; efeitos colaterais/custo caro exigem
  `Lazy<T>` ou lock externo.
- Enumeração do dicionário não é um snapshot atômico; consistência fraca.
- `ConcurrentQueue`: `Count`, `ToArray` e enumeração forçam a criação de novos segmentos e "preservação",
  custando desempenho; segmentos antigos viram lixo (limite de 1M elementos escolhido como "algo arbitrário").
- `ConcurrentBag` sem ordem, e pode reter itens em filas de threads mortas (há mecanismo de recuperação, mas
  o custo de memória/complexidade existe); desempenha mal se produtor ≠ consumidor.
- `BlockingCollection` bloqueia threads — não adequada a código assíncrono em larga escala.
- Estruturas "concorrentes" não tornam composições atômicas: "se não existe, então adiciona" exige as
  operações compostas providas (`GetOrAdd`, `TryUpdate`).
- Atualizações sem alocação para valores atômicos versus nó novo para tipos grandes: custo escondido na
  escolha de `TValue`.

## Como aplicar na sua linguagem

1. **Forneça poucas coleções concorrentes bem escolhidas** e documente o padrão ótimo de cada uma.
2. **Mapa concorrente**: leitura sem lock (publicação com release), escrita com lock por faixa,
   `nível de concorrência = nº de CPUs` por padrão, redimensionamento fora do lock da faixa e com todos os locks.
3. **Operações compostas atômicas** no próprio tipo (`get_or_add`, `compare_and_update`), com o contrato
   explícito de execução do callback fora do lock (e possivelmente repetido) — ou o contrário, se preferir
   garantir exatamente-uma-vez, ao custo de manter o lock durante o callback.
4. **Fila MPMC por sequência de slots** em segmentos encadeados; cache-line padding no par head/tail.
5. **Snapshot** como modo explícito (congelar + preservar), em vez de enumeração "mais ou menos" por padrão.
6. **Evite bloquear no tipo base**: separar a estrutura de dados (sem bloqueio) do decorador bloqueante
   (`BlockingCollection`) e do equivalente assíncrono (Channels, doc 10).
7. **Imutabilidade da tabela por referência** (`volatile Tables`) é uma técnica transferível para qualquer
   estrutura que precise redimensionar sem parar leitores.
8. **Se sua linguagem tem tipos valor grandes**, defina o que é "escrita atômica" (como `IsTValueWriteAtomic`) e
   alocar nó novo em caso contrário.
9. **Teste de stress e verificação de modelo** são essenciais; a documentação do .NET traz comentários extensos
   sobre cada ordem de memória justamente por isso.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentDictionary.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueue.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueueSegment.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/ConcurrentStack.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/ConcurrentBag.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/BlockingCollection.cs`
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/Partitioner.cs`, `OrderablePartitioner.cs`
- `docs/design/specs/Memory-model.md`
