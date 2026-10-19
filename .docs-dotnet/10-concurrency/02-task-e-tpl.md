# Task e TPL (Task Parallel Library)

Como o .NET representa "uma operação que vai terminar" (`Task`/`Task<T>`), como agenda sua execução
(`TaskScheduler`), como encadeia continuações e como expõe paralelismo de dados (`Parallel`).

## Mecanismo no .NET

### `Task` como máquina de estados em um único inteiro

Em `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Task.cs`:

- Toda a vida da task cabe em `internal volatile int m_stateFlags`. A divisão de bits documentada no
  código: `0x7FFF0000` flags de estado, `0x0000FF00` opções internas, `0x000000FF` `TaskCreationOptions`
  públicas.
- Estados terminais: `RanToCompletion`, `Faulted`, `Canceled` (agrupados em `CompletedMask`). Estados
  intermediários: `Started`, `DelegateInvoked`, `WaitingForActivation`, `WaitingOnChildren`,
  `CompletionReserved`, entre outros.
- Mudanças são atômicas via CAS: `AtomicStateUpdate` faz laço com `Interlocked.CompareExchange(ref m_stateFlags, ...)`
  e recusa a transição se bits "proibidos" estiverem ligados (por exemplo, não marcar `Started` se já `Canceled`).
- A flag `CompletionReserved` serve para "reservar" a transição final, de modo que apenas um chamador
  (resultado, exceção ou cancelamento) vença uma corrida.

### Campos raros ficam fora do objeto principal

`ContingentProperties` é alocado só quando necessário (comentário no código: "na maioria dos casos
essas propriedades nunca precisam ser acessadas"). Contém: `ExecutionContext` capturado, evento de
conclusão lazy (`ManualResetEventSlim`, criado só se alguém espera), `TaskExceptionHolder`, token de
cancelamento e seu registro, contador de filhos (`m_completionCountdown`), lista de filhos excepcionais,
pai. Decisão de design: o caminho feliz (`await` sem espera síncrona, sem filhos, sem cancelamento)
não paga por esses campos.

### Continuações: um campo, vários formatos

`private volatile object? m_continuationObject` pode ser: `null`, uma única continuação, uma lista de
continuações, ou o sentinela `s_taskCompletionSentinel` (linhas ~165-170). Fluxo:

- Adicionar continuação: CAS em `m_continuationObject`; se já é o sentinela, a continuação roda/é
  agendada imediatamente (linhas ~4727-4810).
- Completar: `FinishContinuations()` faz `Interlocked.Exchange(ref m_continuationObject, s_taskCompletionSentinel)`
  e executa o que estava lá (linha ~3607). A partir dali, nenhuma continuação é perdida.
- `RunContinuations` trata caso a caso, e o comentário explica que o caso "uma única `IAsyncStateMachineBox`"
  (o `await` típico) é checado cedo por ser o mais comum.
- Inline permitido apenas se a task não tem `RunContinuationsAsynchronously` **e**
  `RuntimeHelpers.TryEnsureSufficientExecutionStack()` for verdadeiro (linhas ~3622-3625): proteção contra
  estouro de pilha em cadeias profundas de continuações síncronas.

### Dois tipos de task

- *Delegate tasks*: têm `m_action`, são agendadas (`Task.Run`, `Task.Factory.StartNew`).
- *Promise tasks* (`InternalTaskOptions.PromiseTask`): representam um resultado futuro sem código
  próprio, ex.: `TaskCompletionSource` (`TaskCompletionSource.cs`, `TaskCompletionSource_T.cs`) e as
  `AsyncStateMachineBox` do `async/await`. `TaskScheduler.TryRunInline` recusa inline de promise tasks
  (checa `task.m_action == null`).

### `TaskScheduler`

`TaskScheduler.cs` define o contrato abstrato:

- `QueueTask(Task)` (abstrato) — enfileirar.
- `TryExecuteTaskInline(Task, bool wasPreviouslyQueued)` (abstrato) — permitir rodar na thread atual.
- `MaximumConcurrencyLevel` (virtual, padrão `int.MaxValue`).
- `TryRunInline` interno valida: não inlina tasks não iniciadas, promise tasks, tasks já invocadas ou
  canceladas, nem quando falta pilha; e lança `InvalidOperationException` se um scheduler customizado
  disser `true` sem que a task tenha realmente rodado ("scheduler com bug").
- `ThreadPoolTaskScheduler` (`ThreadPoolTaskScheduler.cs`): `QueueTask` cria uma thread dedicada para
  `TaskCreationOptions.LongRunning` (`new Thread(...) { IsBackground = true }`), senão chama
  `ThreadPool.UnsafeQueueUserWorkItemInternal(task, preferLocal: (options & PreferFairness) == 0)`.
  Portanto `PreferFairness` desliga o uso da fila local (veja doc 12).
- `FromCurrentSynchronizationContext()` cria um scheduler que posta no `SynchronizationContext` atual.
- `ConcurrentExclusiveSchedulerPair.cs`: par de schedulers estilo reader-writer — tasks "concurrent"
  rodam em paralelo (com limite), tasks "exclusive" rodam sozinhas, ambos sobre um scheduler subjacente.

### Opções de criação relevantes

`TaskCreationOptions`: `AttachedToParent`, `LongRunning`, `DenyChildAttach`, `HideScheduler`,
`PreferFairness`, `RunContinuationsAsynchronously` (valor 0x40). `AttachedToParent` implementa
estruturação: o pai só completa quando os filhos completam (`m_completionCountdown` começa em 1 =
a própria task; o "+1" evita a corrida sobre quem executa a transição final).

### Exceções

`TaskExceptionHolder.cs` agrega exceções; seu finalizador (`~TaskExceptionHolder`) publica
`TaskScheduler.PublishUnobservedTaskException` se ninguém observou a falha — uma forma de não perder
erros silenciosamente. Leitura bloqueante (`Wait`/`Result`) lança `AggregateException`.

### Espera síncrona

`InternalWaitCore` → `SpinThenBlockingWait` (`Task.cs` ~linhas 3119-3162): gira por um tempo antes de
bloquear num `ManualResetEventSlim` criado sob demanda. Além disso, `Wait` tenta antes executar a task
inline no scheduler (`TryRunInline`, ~linhas 1227-1244) se ela ainda não foi iniciada ("task inlining on wait").

### Cache de resultados triviais

`TaskCache.cs` mantém tasks pré-completadas: `s_trueTask`, `s_falseTask` e um array de `Task<int>` para um
intervalo pequeno de inteiros, evitando alocação em `Task.FromResult` e em métodos `async` que retornam
valores comuns.

### Paralelismo de dados (`Parallel`)

`src/libraries/System.Threading.Tasks.Parallel/src/System/Threading/Tasks/`: `Parallel.cs`
(`For`, `ForEach`, `Invoke`), `Parallel.ForEachAsync.cs`, `ParallelLoopState.cs`,
`ParallelRangeManager.cs` (divisão de intervalos), `TaskReplicator.cs` (cria réplicas de uma tarefa que
puxam trabalho até acabar). `ParallelOptions` permite limitar paralelismo e passar `CancellationToken` e
`TaskScheduler`; por padrão usa todos os processadores. Os detalhes finos do particionamento não foram
analisados aqui além da existência desses componentes.

## Por quê

- **Uma só abstração de "futuro"** que serve para trabalho em CPU (delegate tasks), I/O (promise tasks) e
  compor ambos (`WhenAll`, continuações) — base para `async/await` (doc 03).
- **Tamanho e custo**: bit-packing do estado e `ContingentProperties` reduzem tamanho por objeto; como
  tasks são alocadas aos milhões, cada campo importa.
- **Desacoplar "o que" de "onde"**: `TaskScheduler` permite UI-thread, par exclusivo/concorrente, ou
  agendamento customizado, sem mudar o código que cria tasks.
- **Correção sem locks**: transições por CAS + sentinela de completude garantem que continuação adicionada
  durante a conclusão não se perde, e que só um vencedor conclui.
- **Resiliência**: stack-depth guard, detecção de scheduler inconsistente, exceções não observadas.

## Trade-offs

- Tasks são **objetos de heap** (mesmo com otimizações); `ValueTask` (doc 08) existe porque o custo é relevante.
- Modelo rico (filhos, schedulers, opções) aumenta superfície de API e casos de borda
  (`AttachedToParent` raramente é desejável; `HideScheduler` e `DenyChildAttach` existem para corrigir surpresas).
- Inline de continuação é rápido, mas pode rodar código arbitrário em contexto inesperado e exige o guard
  de pilha; `RunContinuationsAsynchronously` evita isso ao custo de uma ida à fila.
- Espera síncrona (`Wait`/`Result`) bloqueia threads do pool: leva a starvation/deadlocks (veja doc 01 e 03).
- Exceções agregadas (`AggregateException`) complicam o tratamento em comparação a uma única exceção.
- O finalizador de exceção não observada depende de GC: o aviso é não-determinístico.

## Como aplicar na sua linguagem

1. **Defina o tipo "futuro" no runtime**, com estado atômico (um inteiro com CAS) e uma única referência
   de continuação que cobre os casos 0/1/N. Sentinela de "completo" evita lock.
2. **Separe delegate de promise** — mas considere unificar a representação; o `AsyncStateMachineBox` do
   .NET é uma `Task<T>` que também guarda a máquina de estados, economizando alocações.
3. **Torne o scheduler pluggável** com poucas operações (enfileirar, tentar inline, nível de concorrência).
4. **Garanta guarda de pilha** para continuações síncronas (ou sempre agende na fila).
5. **Decida cedo sobre concorrência estruturada.** `AttachedToParent` do .NET é opcional e pouco usado;
   uma linguagem nova pode tornar o escopo pai/filho o padrão (inferência de design, não existe no .NET como padrão).
6. **Cache de tasks completadas** para valores comuns (`true`, `false`, inteiros pequenos, unit).
7. **Política para erros não observados**: decidir entre "derrubar o processo", "evento" (.NET) ou
   obrigar a observação (tipo de resultado que o compilador impede de descartar).
8. **Biblioteca de paralelismo de dados** separada do núcleo (`Parallel`), com particionamento dinâmico
   (réplicas que consomem de um range compartilhado) e `CancellationToken` por opção.
9. **Opção `LongRunning`/thread dedicada** para não envenenar o pool.

## Referências

- `src/libraries/System.Private.CoreLib/src/System/Threading/Tasks/Task.cs`
- `.../Tasks/Task_T.cs`, `.../Tasks/TaskCompletionSource.cs`, `.../Tasks/TaskCompletionSource_T.cs`
- `.../Tasks/TaskContinuation.cs`, `.../Tasks/TaskScheduler.cs`, `.../Tasks/ThreadPoolTaskScheduler.cs`
- `.../Tasks/TaskExceptionHolder.cs`, `.../Tasks/TaskCache.cs`, `.../Tasks/ConcurrentExclusiveSchedulerPair.cs`
- `.../Tasks/TaskFactory.cs`, `.../Tasks/ConfigureAwaitOptions.cs`
- `src/libraries/System.Threading.Tasks.Parallel/src/System/Threading/Tasks/Parallel.cs`
- `.../Parallel.ForEachAsync.cs`, `.../ParallelRangeManager.cs`, `.../TaskReplicator.cs`, `.../ParallelLoopState.cs`
