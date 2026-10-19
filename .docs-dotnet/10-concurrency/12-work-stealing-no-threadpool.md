# Work stealing no ThreadPool

Como o ThreadPool do .NET distribui trabalho entre workers: fila local por thread com deque
Chase-Lev-like, filas globais, ordem de busca e as barreiras de memória que tornam isso correto.
Fonte principal: `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadPoolWorkQueue.cs`.

## Mecanismo no .NET

### Estruturas

- **Fila global** `workItems`: `ConcurrentQueue<object>` (alias `WorkQueue` em `#if FEATURE_MULTITHREADING`;
  em builds single-thread o alias é `Queue<object>`). FIFO. Itens são `object`: `Task`,
  `IThreadPoolWorkItem`, `QueueUserWorkItemCallback*`, etc.
- **Fila de alta prioridade** `highPriorityWorkItems` (também `WorkQueue`).
- **Fila de baixa prioridade** `lowPriorityWorkItems` — só usada se `PrioritizationExperiment` está ligada
  (somente CoreCLR; default desligada).
- **Filas globais "atribuíveis"** (`_assignableWorkItemQueues`): só quando `ProcessorCount > 32`, uma por
  grupo de `ProcessorsPerAssignableWorkItemQueue = 16` processadores; cada worker é associado a uma
  (`assignedGlobalWorkItemQueue`) para reduzir contenção na fila global única.
- **Fila local por worker**: `WorkStealingQueue` (classe aninhada), registrada numa lista global
  `WorkStealingQueueList` (array copy-on-write, atualizado com `Interlocked.CompareExchange`).
- **Estado thread-local** `ThreadPoolWorkQueueThreadLocals` (`[ThreadStatic] threadLocals`): contém a
  `workStealingQueue` do worker, um gerador `Random.XoshiroImpl` (para escolher vítimas), flag de alta
  prioridade, índice de fila atribuída e o nó do contador de conclusões. Tem **finalizador** que transfere os
  itens pendentes para a fila global de alta prioridade e remove a fila local da lista.

### `WorkStealingQueue`: o deque

Campos (confirmados):

```
volatile object?[] m_array = new object[INITIAL_SIZE]; // INITIAL_SIZE = 32
volatile int m_mask = INITIAL_SIZE - 1;
volatile int m_headIndex = START_INDEX;   // lado do "ladrão" (steal)
volatile int m_tailIndex = START_INDEX;   // lado do dono (push/pop)
SpinLock m_foreignLock;                   // "lock estrangeiro"
```

Um array circular de potência de 2 indexado por `índice & mask`; índices crescem monotonicamente (em debug
começam em `int.MaxValue` para exercitar a lógica de reset).

**O dono** opera na cauda (LIFO):

- `LocalPush(obj)`: se `tail == int.MaxValue` trata overflow (`LocalPush_HandleTailOverflow`, mascarando os
  índices — "em vez de zerar, mascaramos os bits que não importam", mantendo `head <= tail`); se há espaço de
  ao menos 2 elementos (`tail < m_headIndex + m_mask`) o **caminho rápido não usa lock**: escreve o item no slot e
  então `m_tailIndex = tail + 1` (campo volátil: a publicação só aparece depois do item). Caso contrário pega
  `m_foreignLock` e, se cheio, **dobra** o array (copia a partir de head, reseta head=0, tail=count). No fim,
  `Interlocked.MemoryBarrier()` garante que a verificação de "requisição de thread pendente" feita pelo chamador
  ocorra **depois** do slot aparecer na fila.
- `LocalPop()`: se `head < tail`, `LocalPopCore`: decrementa a cauda com `Interlocked.Exchange(ref m_tailIndex,
  tail)` (fence: a leitura seguinte de `m_headIndex` não pode subir). Se `m_headIndex <= tail` **não há interação
  com ladrões** → pega o item (`Volatile.Read`) sem lock e zera o slot. Se head > tail, a fila estava com 0 ou 1
  elemento e há disputa: entra em `m_foreignLock`, reconfere; se foi roubado, restaura `m_tailIndex = tail + 1` e
  devolve `null`. Slots `null` inesperados provocam `continue` (retry).

**Os ladrões** operam na cabeça (FIFO relativo ao dono):

- `CanSteal => m_headIndex < m_tailIndex` — verificação barata sem lock.
- `TrySteal(ref bool missedSteal)`: `m_foreignLock.TryEnter(ref taken)` — **não bloqueia**; se não conseguiu o
  lock, marca `missedSteal = true`. Com o lock: lê `head`, faz `Interlocked.Exchange(ref m_headIndex, head + 1)`
  (fence: reserva e garante que a leitura de `m_tailIndex` não passe à frente); se `head < m_tailIndex` pega
  `m_array[head & mask]`, zera o slot e retorna; senão **restaura** `m_headIndex = head` via `Interlocked.Exchange`
  (a escrita deve completar antes de retornar com steal perdido e checar requisição pendente).
- Só **um ladrão por vez** por fila (o spinlock), mas dono e ladrão disputam apenas quando restam poucos itens.
- `LocalFindAndPop(obj)` (remoção por referência da fila local do worker atual), exposta via
  `ThreadPool.TryPopCustomWorkItem`, que `ThreadPoolTaskScheduler` (`ThreadPoolTaskScheduler.cs` linhas ~73 e ~92)
  chama ao tentar executar inline/desenfileirar uma task que já estava na fila: só se conseguir tirá-la da fila
  é seguro executá-la no lugar. Também há `Count` (com lock).

### Ordem de busca (`Dequeue`)

Para um worker procurando trabalho (`ThreadPoolWorkQueue.Dequeue(tl, ref missedSteal)`):

1. **Fila local** (`LocalPop`, LIFO — o item mais recente, melhor localidade de cache).
2. **Fila de alta prioridade**, se o worker está em "modo alta prioridade" ou se há itens nela e consegue iniciar
   esse modo (`TryStartProcessingHighPriorityWorkItemsAndDequeue`).
3. **Fila global atribuída** (se há filas atribuíveis).
4. **Fila global** `workItems.TryDequeue`.
5. **Outras filas globais atribuíveis**, a partir de um índice aleatório.
6. **Fila de baixa prioridade** (apenas no experimento).
7. **Roubo**: percorre a lista `WorkStealingQueueList.Queues` começando em um **índice aleatório**
   (`randomValue % c`), pulando a própria fila e filas sem `CanSteal`; retorna o primeiro item roubado.

Se nada é achado e houve `missedSteal`, o worker chama `ThreadPool.EnsureWorkerRequested()` — "falhamos em cumprir
nossa promessa de verificar as filas; precisamos garantir que alguém faça outra passada".

### Quem usa fila local?

`ThreadPoolWorkQueue.Enqueue(callback, forceGlobal)`: se **não** `forceGlobal` e a thread atual é worker
(`threadLocals != null`), faz `LocalPush`; senão enfileira na global (atribuída ou única). `forceGlobal` vem do
parâmetro `preferLocal` das APIs: `QueueUserWorkItem(..., preferLocal: true)` → local; `preferLocal: false`
(padrão de `QueueUserWorkItem(WaitCallback)`) → global. Exemplos confirmados:

- Continuações de `await` e execução de Tasks pelo `ThreadPoolTaskScheduler` usam `preferLocal: true`,
  exceto com `TaskCreationOptions.PreferFairness` (`preferLocal: (options & PreferFairness) == 0`).
- `SynchronizationContext.Post` default usa `preferLocal: false`.
- Isso explica o comportamento típico: uma task que cria sub-tasks as deixa na própria fila local, execução
  em profundidade na mesma thread (LIFO), enquanto outros workers roubam do outro extremo.

### Despacho (`Dispatch`) e integração

- O worker executa em laço até esgotar o **quantum** `DispatchQuantumMs = 30` ms (conferido via
  `Environment.TickCount`) ou a fila; depois `ThreadPool.YieldFromDispatchLoop` pode devolver o controle ao runtime.
- Antes de executar um item o worker chama `EnsureWorkerRequested()` para garantir que, se ainda há trabalho, outro
  worker virá — "o item atual pode bloquear ou depender de progresso de outros".
- Ao ser estacionada/removida pelo hill climbing, a thread **transfere** os itens locais para a fila global de
  alta prioridade (`TransferAllLocalWorkItemsToHighPriorityGlobalQueue`), com tratamento de `OutOfMemoryException`
  (volta para a fila local).
- O flag `_hasOutstandingThreadRequest` (doc 01) limita a uma requisição pendente por vez.
- `Spurious` (`DispatchResult`) indica despacho sem trabalho; a thread pode parar sem girar se há muitos workers
  ativos (`SpuriousDispatchNoSpinThreshold = ProcessorCount * 2 / 3` por padrão).

### Outros users do mesmo padrão

`ConcurrentBag<T>` (doc 07) tem sua própria `WorkStealingQueue` por thread (`TryLocalPop` / `TrySteal`) com a
mesma ideia; `Parallel`/`TaskReplicator` (doc 02) usam o pool para réplicas que puxam de um range compartilhado.

## Por quê

- **Localidade e menor contenção**: a maior parte das operações (push/pop do dono) toca só a fila local, sem lock
  e sem tráfego de cache entre núcleos; só o roubo (raro) atravessa threads.
- **Execução em profundidade (LIFO) do dono, em largura (FIFO) dos ladrões**: o dono continua com dados quentes
  no cache; ladrões levam as tarefas mais antigas, em geral maiores (mais trabalho por roubo).
- **Balanceamento automático** em cargas irregulares (divide-e-conquista, `Parallel`, async fan-out).
- **Evitar gargalo no ponto único**: a fila global única vira o gargalo em máquinas grandes; daí as filas
  atribuíveis acima de 32 processadores.
- **Vítima aleatória** distribui a pressão de roubos e evita convoy.
- **Roubo não bloqueante** (`TryEnter`) e `missedSteal` evitam esperar; em vez disso pede-se outro worker.

## Trade-offs

- **Justiça e latência de cauda**: itens na fila local de um worker ocupado ficam esperando até alguém roubar
  (a próxima rodada pode demorar); `PreferFairness`/`preferLocal:false` existe para quem precisa de FIFO global.
- **Complexidade de concorrência**: o código tem conjuntos delicados de fences (`Interlocked.Exchange` em
  head/tail, `MemoryBarrier` após o push) e um caso de overflow de índice; um erro causa perda ou duplicação de itens
  (o próprio código comenta que um item "removido temporariamente" pode ser perdido de vista por ladrões e por
  isso se assegura requisição de worker).
- **Spinlock estrangeiro** serializa ladrões numa mesma fila.
- **Sync-over-async e inversão de prioridade**: uma thread bloqueada com itens na fila local impede que esses itens
  rodem até alguém roubar; o experimento de priorização e o blocking cooperativo existem por esses cenários.
- **Mais memória e cache-line sharing** por thread (arrays que dobram e não encolhem no código mostrado).
- **Aleatoriedade** dificulta reprodutibilidade (usa `XoshiroImpl` por thread).
- **Roubo de um item por vez**: não há "steal half" (roubo em lote) no código analisado — só um item por
  `TrySteal`.
- **Itens opacos (`object`)**: despacho por teste de tipo em tempo de execução.

## Como aplicar na sua linguagem

1. **Se tiver um scheduler de tarefas, use fila por worker + fila global + roubo**; comece por Chase-Lev ou o
   desenho do .NET (head/tail voláteis + spinlock estrangeiro) conforme sua tolerância a complexidade.
2. **Dono LIFO / ladrão FIFO**, array circular de potência de 2 com crescimento por dobra e índices monotônicos
   (documente a estratégia de overflow de índices).
3. **Defina precisamente as barreiras**: fence entre "decrementar tail" e "ler head" (dono), "incrementar head" e
   "ler tail" (ladrão), e entre "tornar item visível" e "checar se há requisição de worker pendente".
   Use o doc 04 como checklist.
4. **Roubo não bloqueante** e protocolo `missedSteal → pedir outro worker`.
5. **API de agendamento com dica de localidade** (`prefer_local`) e uma via justa (FIFO global) para continuações
   que não devem ficar presas.
6. **Escolha de vítima aleatória** por worker com PRNG barato.
7. **Em máquinas grandes, shardeie a fila global** (grupos de ~16 CPUs, ativado acima de 32) — o .NET faz isso.
8. **Transferência ao aposentar a thread**: nunca perca itens locais quando o worker é parado ou morre
   (finalizador/destrutor + fila global de alta prioridade).
9. **Quantum de execução** (30 ms) para retornar ao runtime, recontar métricas e ajustar o número de workers (doc 01).
10. **Considere roubo em lote** ("steal half") e detecção de bloqueio para evitar a inversão de prioridade — pontos que
    o .NET não resolve completamente (e que seu runtime pode tratar por design, por exemplo com um scheduler que
    conhece operações bloqueantes).

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/ThreadPoolWorkQueue.cs`
  (`WorkStealingQueue`, `WorkStealingQueueList`, `Enqueue`, `Dequeue`, `Dispatch`, `ThreadPoolWorkQueueThreadLocals`)
- `src/libraries/System.Private.CoreLib/src/System/Threading/PortableThreadPool.cs`, `PortableThreadPool.WorkerThread.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/ThreadPoolTaskScheduler.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/TaskContinuation.cs` (`preferLocal: true`)
- `src/libraries/System.Private.CoreLib/src/System/Threading/SynchronizationContext.cs` (`Post` com `preferLocal: false`)
- `src/libraries/System.Collections.Concurrent/src/System/Collections/Concurrent/ConcurrentBag.cs`
- `src/libraries/System.Private.CoreLib/src/System/Collections/Concurrent/ConcurrentQueue.cs`
- `src/libraries/System.Private.CoreLib/src/System/Threading/SpinLock.cs`, `ThreadInt64PersistentCounter.cs`
- `docs/design/specs/Memory-model.md`
